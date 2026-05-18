# Blazing Fast PDF to Markdown Converter

Convert PDF to Markdown with layout detection — preserving images, tables, formulas, captions, headers, and footnotes. Built with Rust, NCNN, and MuPDF for maximum performance.

**Try the free online converter:** [pdf2md.deepdiy.net](https://pdf2md.deepdiy.net/)

## Features

- **Layout-aware Markdown** — Uses DocLayoutNet YOLO-based detection to understand document structure. Output preserves headings, paragraphs, tables, lists, formulas, captions, and more in proper reading order.
- **Images & Assets** — Automatically extracts embedded images and saves them alongside the Markdown output.
- **Clean Output** — No unnecessary line breaks within paragraphs. Produces readable, well-formatted Markdown.
- **Self-hostable** — Pre-built binaries for macOS, Linux, and Windows. No Docker or external services required.
- **Free Web API** — No API key needed. Send a PDF and get back Markdown, image links, and a ZIP download.

## Performance Comparison

![PDF2MD performance comparison vs competitors — 10x faster on a 1c1g VPS](./assets/competitor-comparison.png)
*Faster than other PDF to Markdown tools on equivalent hardware.*

![Self-host on a 1c1g VPS with no Docker required](./assets/self-host-1c1g-vps.png)
*Runs efficiently on a 1-core 1GB RAM VPS.*

![Layout-aware Markdown preserves document structure including tables, lists, and headings](./assets/layout-aware-markdown.png)
*DocLayoutNet detection keeps the original layout intact.*

![Clean Markdown without unnecessary line breaks inside paragraphs](./assets/clean-markdown-no-line-breaks.png)
*No broken inline text — every paragraph stays together.*

![Free web service API for PDF to Markdown conversion](./assets/free-web-service-api.png)
*No sign-up required. Upload and convert instantly.*

## Pre-built Binaries

Download pre-compiled binaries for 4 platforms from the `dist/` directory:

| Platform | Binary |
|----------|--------|
| macOS (Apple Silicon) | `dist/pdf2md-macos-arm64` |
| Linux (x86_64) | `dist/pdf2md-x86_64-unknown-linux-gnu` |
| Linux (ARM64) | `dist/pdf2md-aarch64-unknown-linux-gnu` |
| Windows (x86_64) | `dist/pdf2md-win10-x64.exe` |

### Step 1 — Move files to your working directory

```bash
mv dist/pdf2md-<platform> <workdir>/
mv yolo26n-doclaynet_ncnn_model/ <workdir>/
```

### Step 2 — Run conversion

```bash
cd <workdir>
./pdf2md-<platform> <input.pdf>
```

Export full-page layout input images:

```bash
./pdf2md-<platform> <input.pdf> [output.md] --export-page-image
```

### Arguments

| Argument | Description |
|----------|-------------|
| `input.pdf` | Input PDF file |
| `output.md` | Output Markdown file (optional, defaults to stdout) |

### Extra options

| Option | Description |
|--------|-------------|
| `--asset-dir DIR` | Directory to export page assets |
| `--detect-dpi N` | DPI for layout detection (default: `72`) |
| `--asset-dpi N` | DPI for asset export (default: `150`) |
| `--page N` | Process only the specified page |
| `--model-dir PATH` | Path to the model directory (default: `./yolo26n-doclaynet_ncnn_model/`) |
| `--export-page-image` | Export the full page image used as layout detection input; increase `--detect-dpi` if you need higher-resolution page images |

## Build from Source

```bash
cargo build --release --bin pdf2md
```

The compiled binary will be at `target/release/pdf2md`.

## Run from Source

```bash
cargo run --release --bin pdf2md -- ./input.pdf ./output.md
```

## Self Hosting Streamlit App

A browser-based UI for uploading PDFs and previewing Markdown output with images.

The app automatically detects your OS and architecture to find the right binary in `dist/`. You can also specify a custom path:

```bash
pip install streamlit
streamlit run streamlit_app.py
```

Specify a custom binary or model directory:

```bash
streamlit run streamlit_app.py -- \
  --pdf2md-bin ./dist/pdf2md-<platform> \
  --model-dir /path/to/yolo26n-doclaynet_ncnn_model
```

## Free PDF to Markdown API

No API key required. Submit a PDF and receive Markdown, extracted images, and a downloadable ZIP.

**Endpoint**

```bash
POST https://pdf2md.deepdiy.net/v1/convert
Content-Type: application/pdf
```

**curl example**

```bash
curl -X POST "https://pdf2md.deepdiy.net/v1/convert" \
  -H "Content-Type: application/pdf" \
  --data-binary @paper.pdf
```

**Success response**

```json
{
  "status": "succeeded",
  "markdown": "# Paper title\n\nConverted Markdown...",
  "images": [
    {
      "path": "assets/page_0001_order_0001_class_6.png",
      "url": "https://..."
    }
  ],
  "zip_url": "https://...",
  "download_url": "https://...",
  "expires_in": 300
}
```

**Error response** (HTTP 429)

```json
{
  "error": "busy"
}
```

> The system processes one request at a time across all users. If the server is busy, it returns HTTP 429. Wait 1 second and retry. Each conversion runs for up to 120 seconds — you will likely get a slot within that window.

### API Limits

| Item | Value |
|------|-------|
| Price | Free |
| Max PDF size | 20 MB |
| Concurrency | One request at a time (returns 429 if busy) |
| Max task duration | 120 seconds |
| Conversion timeout | 150 seconds |
| Request timeout | 180 seconds |
| ZIP download expiry | 5 minutes |

## Detection Classes

You can use these class IDs to filter or block specific elements (e.g., Page-header, Footnote) from the output:

`0`: Caption, `1`: Footnote, `2`: Formula, `3`: List-item, `4`: Page-footer, `5`: Page-header, `6`: Picture, `7`: Section-header, `8`: Table, `9`: Text, `10`: Title

## License

MIT License. See [`LICENSE`](./LICENSE).
