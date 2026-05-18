use anyhow::{anyhow, Result};
use std::ffi::{c_void, CString};
use std::path::Path;

type NcnnAllocator = *mut c_void;
type NcnnOption = *mut c_void;
type NcnnMat = *mut c_void;
type NcnnNet = *mut c_void;
type NcnnExtractor = *mut c_void;

const INPUT_NAME: &[u8] = b"in0\0";
const OUTPUT_NAME: &[u8] = b"out0\0";
const IMG_SIZE: u32 = 640;
const CONF_THRESHOLD: f32 = 0.25;
const IOU_THRESHOLD: f32 = 0.45;
const COLORS: [(u8, u8, u8); 11] = [
    (255, 0, 0),
    (0, 255, 0),
    (0, 0, 255),
    (255, 255, 0),
    (255, 0, 255),
    (0, 255, 255),
    (128, 0, 128),
    (0, 128, 255),
    (128, 128, 0),
    (0, 128, 128),
    (128, 0, 0),
];

#[link(name = "ncnn", kind = "static")]
extern "C" {
    fn ncnn_net_create() -> NcnnNet;
    fn ncnn_net_destroy(net: NcnnNet);
    fn ncnn_net_load_param(net: NcnnNet, path: *const i8) -> i32;
    fn ncnn_net_load_model(net: NcnnNet, path: *const i8) -> i32;
    fn ncnn_net_set_option(net: NcnnNet, opt: NcnnOption);
    fn ncnn_extractor_create(net: NcnnNet) -> NcnnExtractor;
    fn ncnn_extractor_destroy(ex: NcnnExtractor);
    fn ncnn_extractor_input(ex: NcnnExtractor, name: *const i8, mat: NcnnMat) -> i32;
    fn ncnn_extractor_extract(ex: NcnnExtractor, name: *const i8, mat: *mut NcnnMat) -> i32;
    fn ncnn_option_create() -> NcnnOption;
    fn ncnn_option_destroy(opt: NcnnOption);
    fn ncnn_option_set_num_threads(opt: NcnnOption, num_threads: i32);
    fn ncnn_option_set_use_vulkan_compute(opt: NcnnOption, enable: i32);
    fn ncnn_option_set_blob_allocator(opt: NcnnOption, allocator: NcnnAllocator);
    fn ncnn_option_set_workspace_allocator(opt: NcnnOption, allocator: NcnnAllocator);
    fn ncnn_allocator_create_pool_allocator() -> NcnnAllocator;
    fn ncnn_allocator_create_unlocked_pool_allocator() -> NcnnAllocator;
    fn ncnn_allocator_destroy(allocator: NcnnAllocator);
    fn ncnn_mat_create_external_3d_elem(
        w: i32,
        h: i32,
        c: i32,
        data: *mut c_void,
        elemsize: usize,
        elempack: i32,
        allocator: NcnnAllocator,
    ) -> NcnnMat;
    fn ncnn_mat_destroy(mat: NcnnMat);
    fn ncnn_mat_get_data(mat: NcnnMat) -> *mut c_void;
    fn ncnn_mat_get_w(mat: NcnnMat) -> i32;
    fn ncnn_mat_get_h(mat: NcnnMat) -> i32;
    fn ncnn_mat_get_c(mat: NcnnMat) -> i32;
}

pub struct Detector {
    net: NcnnNet,
    opt: NcnnOption,
    blob_allocator: NcnnAllocator,
    workspace_allocator: NcnnAllocator,
}

impl Detector {
    pub fn new(model_dir: &Path) -> Result<Self> {
        let param = model_dir.join("model.ncnn.param");
        let bin = model_dir.join("model.ncnn.bin");

        let net = unsafe { ncnn_net_create() };
        if net.is_null() {
            return Err(anyhow!("failed to create ncnn net"));
        }
        let opt = unsafe { ncnn_option_create() };
        if opt.is_null() {
            unsafe { ncnn_net_destroy(net) };
            return Err(anyhow!("failed to create ncnn option"));
        }
        let blob_allocator = unsafe { ncnn_allocator_create_unlocked_pool_allocator() };
        let workspace_allocator = unsafe { ncnn_allocator_create_pool_allocator() };
        if blob_allocator.is_null() || workspace_allocator.is_null() {
            unsafe {
                if !blob_allocator.is_null() {
                    ncnn_allocator_destroy(blob_allocator);
                }
                if !workspace_allocator.is_null() {
                    ncnn_allocator_destroy(workspace_allocator);
                }
                ncnn_option_destroy(opt);
                ncnn_net_destroy(net);
            }
            return Err(anyhow!("failed to create ncnn allocators"));
        }

        unsafe {
            ncnn_option_set_num_threads(opt, 1);
            ncnn_option_set_use_vulkan_compute(opt, 0);
            ncnn_option_set_blob_allocator(opt, blob_allocator);
            ncnn_option_set_workspace_allocator(opt, workspace_allocator);
            ncnn_net_set_option(net, opt);
        }

        if unsafe {
            ncnn_net_load_param(net, cstr(param.to_str().unwrap()).as_ptr() as *const i8)
        } != 0 {
            unsafe {
                ncnn_allocator_destroy(blob_allocator);
                ncnn_allocator_destroy(workspace_allocator);
                ncnn_option_destroy(opt);
                ncnn_net_destroy(net);
            }
            return Err(anyhow!("failed to load param: {}", param.display()));
        }
        if unsafe {
            ncnn_net_load_model(net, cstr(bin.to_str().unwrap()).as_ptr() as *const i8)
        } != 0 {
            unsafe {
                ncnn_allocator_destroy(blob_allocator);
                ncnn_allocator_destroy(workspace_allocator);
                ncnn_option_destroy(opt);
                ncnn_net_destroy(net);
            }
            return Err(anyhow!("failed to load model: {}", bin.display()));
        }

        Ok(Self {
            net,
            opt,
            blob_allocator,
            workspace_allocator,
        })
    }

    pub fn detect_rgb(&self, rgb: &[u8], w: u32, h: u32) -> Result<Detections> {
        let mut input_vec = vec![0f32; (IMG_SIZE * IMG_SIZE * 3) as usize];
        self.detect_rgb_with_buffer(rgb, w, h, &mut input_vec)
    }

    pub fn detect_rgb_with_buffer(
        &self,
        rgb: &[u8],
        w: u32,
        h: u32,
        input_vec: &mut Vec<f32>,
    ) -> Result<Detections> {
        input_vec.resize((IMG_SIZE * IMG_SIZE * 3) as usize, 0.0);
        fill_letterboxed_chw(rgb, w, h, input_vec);

        let ex = unsafe { ncnn_extractor_create(self.net) };
        if ex.is_null() {
            return Err(anyhow!("failed to create extractor"));
        }
        let input_mat = unsafe {
            ncnn_mat_create_external_3d_elem(
                IMG_SIZE as i32,
                IMG_SIZE as i32,
                3,
                input_vec.as_mut_ptr() as *mut c_void,
                std::mem::size_of::<f32>(),
                1,
                std::ptr::null_mut(),
            )
        };
        if input_mat.is_null() {
            unsafe { ncnn_extractor_destroy(ex) };
            return Err(anyhow!("failed to create input mat"));
        }
        if unsafe { ncnn_extractor_input(ex, INPUT_NAME.as_ptr() as *const i8, input_mat) } != 0 {
            unsafe {
                ncnn_mat_destroy(input_mat);
                ncnn_extractor_destroy(ex);
            }
            return Err(anyhow!("failed to feed input"));
        }

        let mut output_mat: NcnnMat = std::ptr::null_mut();
        if unsafe { ncnn_extractor_extract(ex, OUTPUT_NAME.as_ptr() as *const i8, &mut output_mat) }
            != 0
            || output_mat.is_null()
        {
            unsafe {
                ncnn_mat_destroy(input_mat);
                ncnn_extractor_destroy(ex);
            }
            return Err(anyhow!("failed to extract output"));
        }

        let ow = unsafe { ncnn_mat_get_w(output_mat) } as usize;
        let oh = unsafe { ncnn_mat_get_h(output_mat) } as usize;
        let oc = unsafe { ncnn_mat_get_c(output_mat) } as usize;
        let len = ow.max(1) * oh.max(1) * oc.max(1);
        let raw =
            unsafe { std::slice::from_raw_parts(ncnn_mat_get_data(output_mat) as *const f32, len) };
        let detections = postprocess(raw, w, h);

        unsafe {
            ncnn_mat_destroy(input_mat);
            ncnn_mat_destroy(output_mat);
            ncnn_extractor_destroy(ex);
        }

        Ok(detections)
    }
}

impl Drop for Detector {
    fn drop(&mut self) {
        unsafe {
            ncnn_net_destroy(self.net);
            ncnn_option_destroy(self.opt);
            ncnn_allocator_destroy(self.blob_allocator);
            ncnn_allocator_destroy(self.workspace_allocator);
        }
    }
}

pub struct Detections {
    pub boxes: Vec<[f32; 4]>,
    pub scores: Vec<f32>,
    pub class_ids: Vec<i32>,
}

pub fn draw(image_bgr: &mut image::RgbImage, detections: &Detections) {
    for ((box_, score), cls_id) in detections
        .boxes
        .iter()
        .zip(&detections.scores)
        .zip(&detections.class_ids)
    {
        let cls = (*cls_id).clamp(0, (COLORS.len() - 1) as i32) as usize;
        let (b, g, r) = COLORS[cls];
        let _ = score;
        let x1 = box_[0].round().max(0.0) as u32;
        let y1 = box_[1].round().max(0.0) as u32;
        let x2 = box_[2].round().max(0.0) as u32;
        let y2 = box_[3].round().max(0.0) as u32;
        draw_rect(image_bgr.as_mut(), x1, y1, x2, y2, [b, g, r]);
    }
}

pub fn draw_rgb_bytes(image: &mut [u8], width: u32, height: u32, detections: &Detections) {
    for ((box_, score), cls_id) in detections
        .boxes
        .iter()
        .zip(&detections.scores)
        .zip(&detections.class_ids)
    {
        let cls = (*cls_id).clamp(0, (COLORS.len() - 1) as i32) as usize;
        let (b, g, r) = COLORS[cls];
        let _ = score;
        let x1 = box_[0].round().max(0.0) as u32;
        let y1 = box_[1].round().max(0.0) as u32;
        let x2 = box_[2].round().max(0.0) as u32;
        let y2 = box_[3].round().max(0.0) as u32;
        draw_rect_rgb_bytes(image, width, height, x1, y1, x2, y2, [b, g, r]);
    }
}

pub fn detect_image_file(image_path: &Path, model_dir: &Path) -> Result<Detections> {
    let detector = Detector::new(model_dir)?;
    let image = open_image_file(image_path)?;
    detector.detect_rgb(image.as_raw(), image.width(), image.height())
}

pub fn open_image_file(image_path: &Path) -> Result<image::RgbImage> {
    image::open(image_path)
        .map_err(|e| anyhow!("failed to open image {}: {e}", image_path.display()))?
        .to_rgb8()
        .pipe(Ok)
}

fn postprocess(raw_flat: &[f32], orig_w: u32, orig_h: u32) -> Detections {
    let pred = reshape_15x_n(raw_flat);
    let mut boxes = Vec::new();
    let mut scores = Vec::new();
    let mut class_ids = Vec::new();
    for row in pred {
        let x = row[0];
        let y = row[1];
        let w = row[2];
        let h = row[3];
        let cls_slice = &row[4..];
        let (cls_id, score) = cls_slice
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, s)| (i as i32, *s))
            .unwrap_or((0, 0.0));
        if score < CONF_THRESHOLD {
            continue;
        }
        boxes.push([x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0]);
        scores.push(score);
        class_ids.push(cls_id);
    }
    let scale = (IMG_SIZE as f32 / orig_w as f32).min(IMG_SIZE as f32 / orig_h as f32);
    let new_w = (orig_w as f32 * scale).round();
    let new_h = (orig_h as f32 * scale).round();
    let pad_x = ((IMG_SIZE as f32) - new_w) / 2.0;
    let pad_y = ((IMG_SIZE as f32) - new_h) / 2.0;
    for b in &mut boxes {
        b[0] = ((b[0] - pad_x) / scale).clamp(0.0, orig_w as f32);
        b[2] = ((b[2] - pad_x) / scale).clamp(0.0, orig_w as f32);
        b[1] = ((b[1] - pad_y) / scale).clamp(0.0, orig_h as f32);
        b[3] = ((b[3] - pad_y) / scale).clamp(0.0, orig_h as f32);
    }
    let mut keep = Vec::new();
    let mut unique = class_ids.clone();
    unique.sort_unstable();
    unique.dedup();
    for cls_id in unique {
        let indices: Vec<usize> = class_ids
            .iter()
            .enumerate()
            .filter_map(|(i, &c)| if c == cls_id { Some(i) } else { None })
            .collect();
        let mut cls_boxes = Vec::with_capacity(indices.len());
        let mut cls_scores = Vec::with_capacity(indices.len());
        for &idx in &indices {
            cls_boxes.push(boxes[idx]);
            cls_scores.push(scores[idx]);
        }
        for idx in nms(&cls_boxes, &cls_scores) {
            keep.push(indices[idx]);
        }
    }
    keep.sort_by(|&a, &b| scores[b].partial_cmp(&scores[a]).unwrap());
    Detections {
        boxes: keep.iter().map(|&i| boxes[i]).collect(),
        scores: keep.iter().map(|&i| scores[i]).collect(),
        class_ids: keep.iter().map(|&i| class_ids[i]).collect(),
    }
}

fn reshape_15x_n(raw_flat: &[f32]) -> Vec<[f32; 15]> {
    let n = raw_flat.len() / 15;
    let mut rows = vec![[0.0f32; 15]; n];
    for c in 0..15 {
        let base = c * n;
        for i in 0..n {
            rows[i][c] = raw_flat[base + i];
        }
    }
    rows
}

fn nms(boxes: &[[f32; 4]], scores: &[f32]) -> Vec<usize> {
    if boxes.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| scores[b].partial_cmp(&scores[a]).unwrap());
    let mut keep = Vec::new();
    while let Some(&i) = order.first() {
        keep.push(i);
        if order.len() == 1 {
            break;
        }
        let mut next = Vec::with_capacity(order.len() - 1);
        for &j in &order[1..] {
            if iou(boxes[i], boxes[j]) <= IOU_THRESHOLD {
                next.push(j);
            }
        }
        order = next;
    }
    keep
}

fn iou(a: [f32; 4], b: [f32; 4]) -> f32 {
    let x1 = a[0].max(b[0]);
    let y1 = a[1].max(b[1]);
    let x2 = a[2].min(b[2]);
    let y2 = a[3].min(b[3]);
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let area_a = (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0);
    let area_b = (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
    inter / (area_a + area_b - inter).max(1e-9)
}

fn fill_letterboxed_chw(rgb: &[u8], w: u32, h: u32, out: &mut [f32]) {
    let scale = (IMG_SIZE as f32 / w as f32).min(IMG_SIZE as f32 / h as f32);
    let new_w = (w as f32 * scale).round() as u32;
    let new_h = (h as f32 * scale).round() as u32;
    let pad_x = (IMG_SIZE - new_w) / 2;
    let pad_y = (IMG_SIZE - new_h) / 2;
    let scale_x = w as f32 / new_w as f32;
    let scale_y = h as f32 / new_h as f32;
    let plane = (IMG_SIZE * IMG_SIZE) as usize;
    out.fill(114.0 / 255.0);
    for dy in 0..new_h {
        let fy = (dy as f32 + 0.5) * scale_y - 0.5;
        let y0f = fy.floor();
        let wy = fy - y0f;
        let y0 = y0f as i32;
        let y1 = y0 + 1;
        let y0c = y0.clamp(0, h as i32 - 1) as u32;
        let y1c = y1.clamp(0, h as i32 - 1) as u32;
        for dx in 0..new_w {
            let fx = (dx as f32 + 0.5) * scale_x - 0.5;
            let x0f = fx.floor();
            let wx = fx - x0f;
            let x0 = x0f as i32;
            let x1 = x0 + 1;
            let x0c = x0.clamp(0, w as i32 - 1) as u32;
            let x1c = x1.clamp(0, w as i32 - 1) as u32;
            let p00 = rgb_pixel(rgb, w, x0c, y0c);
            let p01 = rgb_pixel(rgb, w, x1c, y0c);
            let p10 = rgb_pixel(rgb, w, x0c, y1c);
            let p11 = rgb_pixel(rgb, w, x1c, y1c);
            let out_x = (pad_x + dx) as usize;
            let out_y = (pad_y + dy) as usize;
            let idx = out_y * IMG_SIZE as usize + out_x;
            for ch in 0..3 {
                let top = p00[ch] as f32 * (1.0 - wx) + p01[ch] as f32 * wx;
                let bottom = p10[ch] as f32 * (1.0 - wx) + p11[ch] as f32 * wx;
                out[ch * plane + idx] = (top * (1.0 - wy) + bottom * wy) / 255.0;
            }
        }
    }
}

fn rgb_pixel(rgb: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let idx = ((y * width + x) * 3) as usize;
    [rgb[idx], rgb[idx + 1], rgb[idx + 2]]
}

fn draw_rect(img: &mut [u8], x1: u32, y1: u32, x2: u32, y2: u32, color: [u8; 3]) {
    let pixels = img.len() / 3;
    let w = (pixels as f32).sqrt() as u32;
    let h = if w > 0 { (pixels as u32) / w } else { 0 };
    draw_rect_rgb_bytes(img, w, h, x1, y1, x2, y2, color);
}

fn draw_rect_rgb_bytes(
    img: &mut [u8],
    width: u32,
    height: u32,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    color: [u8; 3],
) {
    let x1 = x1.min(width.saturating_sub(1));
    let y1 = y1.min(height.saturating_sub(1));
    let x2 = x2.min(width.saturating_sub(1));
    let y2 = y2.min(height.saturating_sub(1));
    for x in x1..=x2 {
        if y1 < height {
            put_pixel_rgb(img, width, x, y1, color);
        }
        if y2 < height {
            put_pixel_rgb(img, width, x, y2, color);
        }
    }
    for y in y1..=y2 {
        if x1 < width {
            put_pixel_rgb(img, width, x1, y, color);
        }
        if x2 < width {
            put_pixel_rgb(img, width, x2, y, color);
        }
    }
}

fn put_pixel_rgb(img: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 3]) {
    let idx = ((y * width + x) * 3) as usize;
    if idx + 2 < img.len() {
        img[idx] = color[0];
        img[idx + 1] = color[1];
        img[idx + 2] = color[2];
    }
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap()
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}
