use anyhow::{anyhow, Context, Result};
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use mupdf::{
    text_page::TextBlockType, Colorspace, Device, Document, IRect, Matrix, Pixmap, TextPageFlags,
};
use std::cmp::Ordering;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use pdf2md_ncnn_rust::{Detections, Detector};

const CLASS_CAPTION: i32 = 0;
const CLASS_FOOTNOTE: i32 = 1;
const CLASS_LIST_ITEM: i32 = 3;
const CLASS_PAGE_FOOTER: i32 = 4;
const CLASS_SECTION_HEADER: i32 = 7;
const CLASS_TEXT: i32 = 9;
const CLASS_TITLE: i32 = 10;

#[derive(Clone)]
struct TextLine {
    text: String,
    bbox: [f32; 4],
}

#[derive(Clone, Copy)]
struct LayoutBox {
    idx: usize,
    bbox: [f32; 4],
    class_id: i32,
}

struct Paragraph {
    class_id: i32,
    order_index: usize,
    box_index: usize,
    bbox: [f32; 4],
    text: String,
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let pdf_path = PathBuf::from(args.next().ok_or_else(|| {
        anyhow!("usage: pdf2md <input.pdf> [output.md] [--asset-dir DIR] [--detect-dpi N] [--asset-dpi N] [--page N] [--model-dir PATH] [--export-page-image]")
    })?);
    let output_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output_pdf2md.md"));
    let mut asset_dir = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| PathBuf::from(format!("{s}_assets")))
        .unwrap_or_else(|| PathBuf::from("pdf2md_assets"));
    let mut detect_dpi = 72.0f32;
    let mut asset_dpi = 150.0f32;
    let mut page_filter: Option<usize> = None;
    let mut export_page_image = false;
    let mut model_dir = std::env::current_dir()?.join("yolo26n-doclaynet_ncnn_model");

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--asset-dir" => {
                asset_dir = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--asset-dir requires a value"))?,
                );
            }
            "--detect-dpi" => {
                detect_dpi = args
                    .next()
                    .ok_or_else(|| anyhow!("--detect-dpi requires a value"))?
                    .parse::<f32>()?;
            }
            "--asset-dpi" => {
                asset_dpi = args
                    .next()
                    .ok_or_else(|| anyhow!("--asset-dpi requires a value"))?
                    .parse::<f32>()?;
            }
            "--page" => {
                page_filter = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--page requires a value"))?
                        .parse::<usize>()?,
                );
            }
            "--model-dir" => {
                model_dir = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--model-dir requires a value"))?,
                );
            }
            "--export-page-image" => export_page_image = true,
            other => return Err(anyhow!("unknown flag: {other}")),
        }
    }

    if !pdf_path.exists() {
        return Err(anyhow!("PDF not found: {}", pdf_path.display()));
    }

    std::fs::create_dir_all(&asset_dir)
        .with_context(|| format!("failed to create asset dir: {}", asset_dir.display()))?;

    let detector = Detector::new(&model_dir)?;
    let pdf_path_for_mupdf = pdf_path.to_string_lossy().into_owned();
    let document = Document::open(&pdf_path_for_mupdf)
        .with_context(|| format!("failed to open PDF: {}", pdf_path.display()))?;
    let page_count = document.page_count()? as usize;
    let colorspace = Colorspace::device_rgb();
    let detect_matrix = Matrix::new_scale(detect_dpi / 72.0, detect_dpi / 72.0);
    let mut out = BufWriter::new(File::create(&output_path)?);
    let mut detector_input = Vec::new();

    for page_num in 1..=page_count {
        if page_filter.is_some_and(|p| p != page_num) {
            continue;
        }

        let page = document.load_page((page_num - 1) as i32)?;
        let detect_pixmap = page.to_pixmap(&detect_matrix, &colorspace, false, false)?;
        let detect_w = detect_pixmap.width() as u32;
        let detect_h = detect_pixmap.height() as u32;
        let detections = detector.detect_rgb_with_buffer(
            detect_pixmap.samples(),
            detect_w,
            detect_h,
            &mut detector_input,
        )?;
        if export_page_image {
            save_detect_page_image(&detect_pixmap, page_num, &asset_dir)?;
        }
        // detect pixmap no longer needed after inference
        drop(detect_pixmap);

        let lines = extract_lines(&page, detect_dpi / 72.0)?;
        let paragraphs = build_paragraphs(&detections, &lines, detect_w as f32, detect_h as f32);

        write_page_markdown(
            &mut out,
            page_num,
            &paragraphs,
            &page,
            &colorspace,
            detect_dpi,
            asset_dpi,
            &asset_dir,
            &output_path,
        )?;

        drop(page);
        println!("page {page_num}/{page_count}: {} boxes", paragraphs.len());
    }

    out.flush()?;
    println!("markdown: {}", output_path.display());
    println!("assets: {}", asset_dir.display());
    Ok(())
}

fn save_detect_page_image(detect_pixmap: &Pixmap, page_num: usize, asset_dir: &Path) -> Result<()> {
    let path = asset_dir.join(format!("page_{:04}_detect_input.png", page_num));
    let file =
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    PngEncoder::new(file)
        .write_image(
            detect_pixmap.samples(),
            detect_pixmap.width() as u32,
            detect_pixmap.height() as u32,
            ColorType::Rgb8.into(),
        )
        .with_context(|| format!("failed to save {}", path.display()))?;
    Ok(())
}

fn extract_lines(page: &mupdf::Page, scale: f32) -> Result<Vec<TextLine>> {
    let text_page = page.to_text_page(TextPageFlags::PRESERVE_WHITESPACE)?;
    let mut lines = Vec::new();

    for block in text_page.blocks() {
        if block.r#type() != TextBlockType::Text {
            continue;
        }
        for line in block.lines() {
            let text: String = line.chars().filter_map(|c| c.char()).collect();
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            let b = line.bounds();
            lines.push(TextLine {
                text,
                bbox: [b.x0 * scale, b.y0 * scale, b.x1 * scale, b.y1 * scale],
            });
        }
    }

    lines.sort_by(|a, b| {
        cmp_f32(&a.bbox[1], &b.bbox[1]).then_with(|| cmp_f32(&a.bbox[0], &b.bbox[0]))
    });
    Ok(lines)
}

fn build_paragraphs(
    detections: &Detections,
    lines: &[TextLine],
    page_w: f32,
    page_h: f32,
) -> Vec<Paragraph> {
    let (order_by_box, ordered_boxes) = reading_order(detections, page_w, page_h);
    let mut grouped_lines: Vec<Vec<&TextLine>> =
        (0..detections.boxes.len()).map(|_| Vec::new()).collect();

    for line in lines {
        if let Some(idx) = best_box_for_line(line.bbox, &detections.boxes) {
            grouped_lines[idx].push(line);
        }
    }

    let mut paragraphs = Vec::with_capacity(ordered_boxes.len());
    for box_index in ordered_boxes {
        let grouped = &mut grouped_lines[box_index];
        grouped.sort_by(|a, b| {
            cmp_f32(&a.bbox[1], &b.bbox[1]).then_with(|| cmp_f32(&a.bbox[0], &b.bbox[0]))
        });
        let text = grouped
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        paragraphs.push(Paragraph {
            class_id: detections.class_ids[box_index],
            order_index: order_by_box[box_index],
            box_index,
            bbox: detections.boxes[box_index],
            text,
        });
    }
    paragraphs
}

fn write_page_markdown<W: Write>(
    out: &mut W,
    page_num: usize,
    paragraphs: &[Paragraph],
    page: &mupdf::Page,
    colorspace: &mupdf::Colorspace,
    detect_dpi: f32,
    asset_dpi: f32,
    asset_dir: &Path,
    output_path: &Path,
) -> Result<()> {
    for para in paragraphs {
        match para.class_id {
            CLASS_TITLE => write_text_block(out, "#", &para.text)?,
            CLASS_SECTION_HEADER => write_text_block(out, "##", &para.text)?,
            CLASS_TEXT | CLASS_LIST_ITEM | CLASS_CAPTION => write_text_block(out, "", &para.text)?,
            CLASS_FOOTNOTE => write_quote_block(out, "Footnote", &para.text)?,
            CLASS_PAGE_FOOTER => write_footer_block(out, &para.text)?,
            _ => {
                let image_path = save_box_image(
                    page, colorspace, page_num, para, detect_dpi, asset_dpi, asset_dir,
                )?;
                let rel = relative_path(
                    &image_path,
                    output_path.parent().unwrap_or_else(|| Path::new(".")),
                );
                writeln!(
                    out,
                    "\n![page {} box {}]({})\n",
                    page_num,
                    para.box_index,
                    rel.display()
                )?;
            }
        }
    }
    Ok(())
}

fn write_footer_block<W: Write>(out: &mut W, text: &str) -> Result<()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    writeln!(out, "\n---\n{text}\n\n---\n")?;
    Ok(())
}

fn write_quote_block<W: Write>(out: &mut W, label: &str, text: &str) -> Result<()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    writeln!(out, "\n> **{label}:** {}\n", text.replace('\n', "\n> "))?;
    Ok(())
}

fn write_text_block<W: Write>(out: &mut W, prefix: &str, text: &str) -> Result<()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    if prefix.is_empty() {
        writeln!(out, "\n{text}\n")?;
    } else {
        writeln!(out, "\n{prefix} {text}\n")?;
    }
    Ok(())
}

fn save_box_image(
    page: &mupdf::Page,
    colorspace: &mupdf::Colorspace,
    page_num: usize,
    para: &Paragraph,
    detect_dpi: f32,
    asset_dpi: f32,
    asset_dir: &Path,
) -> Result<PathBuf> {
    let [x0, y0, x1, y1] = para.bbox;
    let pad_detect = 3.0f32; // 3-pixel pad in detect space
    let ppd = 72.0 / detect_dpi; // points per detect-pixel

    // Page-space bbox (with pad)
    let px0 = (x0 - pad_detect) * ppd;
    let py0 = (y0 - pad_detect) * ppd;
    let px1 = (x1 + pad_detect) * ppd;
    let py1 = (y1 + pad_detect) * ppd;

    // Output pixmap dimensions in asset pixels
    let scale = asset_dpi / 72.0;
    let out_w = ((px1 - px0) * scale).ceil().max(1.0) as i32;
    let out_h = ((py1 - py0) * scale).ceil().max(1.0) as i32;

    if out_w < 2 || out_h < 2 {
        return Err(anyhow!("crop region too small: {out_w}x{out_h}"));
    }

    // Create a pixmap for just the crop region
    let mut local_pixmap = Pixmap::new_with_w_h(colorspace, out_w, out_h, false)
        .with_context(|| "failed to create local pixmap")?;
    local_pixmap
        .clear_with(255)
        .with_context(|| "failed to clear pixmap")?;

    // Create a draw device bound to this pixmap, clipped to its full extent
    let clip = IRect::new(0, 0, out_w, out_h);
    let device = Device::from_pixmap_with_clip(&local_pixmap, clip)
        .with_context(|| "failed to create draw device")?;

    // Matrix: page space → device pixel space.
    //   output_x = (page_x - px0) * scale
    //   output_y = (page_y - py0) * scale
    let ctm = Matrix::new(
        scale,
        0.0, // a b
        0.0,
        scale, // c d
        -px0 * scale,
        -py0 * scale, // e f
    );

    page.run(&device, &ctm)
        .with_context(|| "failed to render page region")?;

    // Device must be dropped before reading pixmap samples
    drop(device);

    let samples = local_pixmap.samples();
    let width = out_w as u32;
    let height = out_h as u32;

    let path = asset_dir.join(format!(
        "page_{:04}_order_{:04}_class_{}.png",
        page_num, para.order_index, para.class_id
    ));
    let file =
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    PngEncoder::new(file)
        .write_image(samples, width, height, ColorType::Rgb8.into())
        .with_context(|| format!("failed to save {}", path.display()))?;
    Ok(path)
}

fn reading_order(detections: &Detections, page_w: f32, page_h: f32) -> (Vec<usize>, Vec<usize>) {
    let mut headers = Vec::new();
    let mut body = Vec::new();
    let mut footers = Vec::new();

    for idx in 0..detections.boxes.len() {
        let item = LayoutBox {
            idx,
            bbox: detections.boxes[idx],
            class_id: detections.class_ids[idx],
        };
        if item.class_id == 5 {
            headers.push(item);
        } else if item.class_id == 4 || item.class_id == 1 {
            footers.push(item);
        } else if item.bbox[3] > page_h * 0.97 && item.class_id != 10 && item.class_id != 7 {
            footers.push(item);
        } else {
            body.push(item);
        }
    }

    sort_yx(&mut headers);
    sort_yx(&mut footers);

    let mut ordered = headers;
    ordered.extend(order_body(body, page_w));
    ordered.extend(footers);

    let mut order_by_box = vec![0; detections.boxes.len()];
    let mut ordered_boxes = Vec::with_capacity(ordered.len());
    for (order_index, item) in ordered.iter().enumerate() {
        order_by_box[item.idx] = order_index;
        ordered_boxes.push(item.idx);
    }

    (order_by_box, ordered_boxes)
}

fn order_body(mut body: Vec<LayoutBox>, page_w: f32) -> Vec<LayoutBox> {
    sort_yx(&mut body);

    let mut ordered = Vec::new();
    let mut zone = Vec::new();
    let mut zone_bottom = 0.0f32;

    for item in body {
        if is_zone_separator(item, page_w) {
            ordered.extend(order_zone(zone, page_w));
            zone = Vec::new();
            ordered.push(item);
            zone_bottom = item.bbox[3];
            continue;
        }

        if !zone.is_empty() && item.bbox[1] - zone_bottom > vertical_gap_threshold(page_w) {
            ordered.extend(order_zone(zone, page_w));
            zone = Vec::new();
        }

        zone_bottom = zone_bottom.max(item.bbox[3]);
        zone.push(item);
    }

    ordered.extend(order_zone(zone, page_w));
    ordered
}

fn order_zone(mut zone: Vec<LayoutBox>, page_w: f32) -> Vec<LayoutBox> {
    if zone.len() <= 2 || !looks_two_column(&zone, page_w) {
        sort_yx(&mut zone);
        return zone;
    }

    let mut left = Vec::new();
    let mut right = Vec::new();

    for item in zone {
        if center_x(item.bbox) < page_w * 0.5 {
            left.push(item);
        } else {
            right.push(item);
        }
    }

    sort_yx(&mut left);
    sort_yx(&mut right);
    left.extend(right);
    left
}

fn looks_two_column(items: &[LayoutBox], page_w: f32) -> bool {
    let mut widths: Vec<f32> = items
        .iter()
        .filter(|item| !is_full_width(**item, page_w))
        .map(|item| item.bbox[2] - item.bbox[0])
        .collect();
    if widths.len() < 4 {
        return false;
    }

    widths.sort_by(cmp_f32);
    let median_width = widths[widths.len() / 2];
    let narrow_enough = median_width < page_w * 0.55;
    let left_count = items
        .iter()
        .filter(|item| center_x(item.bbox) < page_w * 0.48)
        .count();
    let right_count = items
        .iter()
        .filter(|item| center_x(item.bbox) > page_w * 0.52)
        .count();

    narrow_enough && left_count >= 2 && right_count >= 2
}

fn is_full_width(item: LayoutBox, page_w: f32) -> bool {
    let width = item.bbox[2] - item.bbox[0];
    width >= page_w * 0.60
        || ((item.class_id == 10 || item.class_id == 7) && width >= page_w * 0.45)
}

fn is_zone_separator(item: LayoutBox, page_w: f32) -> bool {
    is_full_width(item, page_w)
        || ((item.class_id == 10 || item.class_id == 7)
            && center_x(item.bbox) > page_w * 0.35
            && center_x(item.bbox) < page_w * 0.65)
}

fn vertical_gap_threshold(page_w: f32) -> f32 {
    (page_w * 0.035).clamp(18.0, 36.0)
}

fn sort_yx(items: &mut [LayoutBox]) {
    items.sort_by(|a, b| {
        cmp_f32(&a.bbox[1], &b.bbox[1]).then_with(|| cmp_f32(&a.bbox[0], &b.bbox[0]))
    });
}

fn cmp_f32(a: &f32, b: &f32) -> Ordering {
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

fn center_x(b: [f32; 4]) -> f32 {
    (b[0] + b[2]) * 0.5
}

fn best_box_for_line(line: [f32; 4], boxes: &[[f32; 4]]) -> Option<usize> {
    boxes
        .iter()
        .enumerate()
        .filter(|(_, bbox)| center_inside(line, **bbox))
        .min_by(|(_, a), (_, b)| box_area(**a).partial_cmp(&box_area(**b)).unwrap())
        .map(|(idx, _)| idx)
}

fn center_inside(line: [f32; 4], bbox: [f32; 4]) -> bool {
    let cx = (line[0] + line[2]) * 0.5;
    let cy = (line[1] + line[3]) * 0.5;
    cx >= bbox[0] && cx <= bbox[2] && cy >= bbox[1] && cy <= bbox[3]
}

fn box_area(b: [f32; 4]) -> f32 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

#[allow(dead_code)]
fn relative_path(path: &Path, base: &Path) -> PathBuf {
    path.strip_prefix(base).unwrap_or(path).to_path_buf()
}
