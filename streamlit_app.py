from __future__ import annotations

import argparse
import io
import os
import platform
import re
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

import streamlit as st


def _default_bin() -> str:
    """Detect platform and return the expected binary name in dist/."""
    root = Path(__file__).resolve().parent / "dist"
    sys_ = sys.platform
    machine = platform.machine()
    if sys_ == "darwin":
        name = "pdf2md-macos-arm64" if machine == "arm64" else "pdf2md-macos-x86_64"
    elif sys_ == "win32":
        name = "pdf2md-win10-x64.exe"
    elif sys_ == "linux":
        if machine == "aarch64":
            name = "pdf2md-aarch64-unknown-linux-gnu"
        else:
            name = "pdf2md-x86_64-unknown-linux-gnu"
    else:
        name = "pdf2md"
    return str(root / name)


IMG_RE = re.compile(r"(!\[([^\]]*)\]\(([^)]+)\))")


def convert(
    bin: Path, pdf: Path, md: Path, assets: Path, model_dir: Path | None
) -> None:
    cmd = [str(bin), str(pdf), str(md), "--asset-dir", str(assets)]
    if model_dir is not None:
        cmd += ["--model-dir", str(model_dir)]
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    assert proc.stdout is not None
    lines = []
    output = st.empty()
    for line in iter(proc.stdout.readline, ""):
        line = line.rstrip("\n")
        if line:
            lines.append(line)
            output.code("\n".join(lines[-5:]), language="")
    proc.wait()
    if proc.returncode != 0:
        err = proc.stderr.read() if proc.stderr else ""
        raise subprocess.CalledProcessError(proc.returncode, proc.args, stderr=err)


def render(md: str, md_dir: Path) -> None:
    for text, alt, rel in IMG_RE.findall(md):
        before, _, md = md.partition(text)
        if before.strip():
            st.markdown(before)
        img = (md_dir / rel).resolve()
        if img.exists():
            st.image(str(img), caption=alt or None)
    if md.strip():
        st.markdown(md)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="PDF to Markdown — Self-hosted Streamlit UI"
    )
    parser.add_argument(
        "--pdf2md-bin",
        default=os.environ.get("PDF2MD_BIN", _default_bin()),
        help="Path to the pdf2md binary (env: PDF2MD_BIN)",
    )
    parser.add_argument(
        "--model-dir",
        default=os.environ.get("PDF2MD_MODEL_DIR"),
        help="Path to the model directory (env: PDF2MD_MODEL_DIR)",
    )
    args, _ = parser.parse_known_args()

    pdf2md = Path(args.pdf2md_bin).resolve()
    model_dir = Path(args.model_dir).resolve() if args.model_dir else None

    if not pdf2md.exists():
        st.error(f"pdf2md binary not found at: {pdf2md}")
        st.info("Set --pdf2md-bin or the PDF2MD_BIN environment variable.")
        return

    st.set_page_config(page_title="PDF → Markdown", page_icon="📄", layout="centered")
    st.title("📄 PDF → Markdown")

    uploaded = st.file_uploader("Select a PDF file", type=["pdf"])
    if not uploaded:
        st.info("Please upload a PDF file")
        return

    with tempfile.TemporaryDirectory() as tmpdir:
        tmp = Path(tmpdir)
        pdf = tmp / uploaded.name
        pdf.write_bytes(uploaded.read())
        md = tmp / "output.md"

        with st.status("⏳ Converting...", expanded=True) as status:
            try:
                convert(pdf2md, pdf, md, tmp / "assets", model_dir)
                status.update(label="✅ Conversion complete", state="complete")
            except subprocess.CalledProcessError as e:
                st.error(f"Conversion failed:\n```\n{e.stderr}\n```")
                return
            except FileNotFoundError:
                st.error("Conversion tool not found")
                return

        if not md.exists():
            st.error("Conversion failed: Markdown file was not generated")
            return

        content = md.read_text("utf-8")
        imgs = sorted({m.group(3) for m in IMG_RE.finditer(content)})
        name = f"{uploaded.name.rsplit('.', 1)[0]}.md"

        buf = io.BytesIO()
        with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
            zf.writestr(name, content.encode("utf-8"))
            for rel in imgs:
                p = (tmp / rel).resolve()
                if p.exists():
                    zf.write(p, rel)

        st.divider()
        tab1, tab2 = st.tabs(["📝 Rendered Preview", "📄 Raw Markdown"])
        with tab1:
            render(content, tmp)
        with tab2:
            st.code(content, language="markdown")

        col1, col2 = st.columns(2)
        with col1:
            st.download_button(
                "⬇️ Download .md",
                content,
                name,
                "text/markdown",
                use_container_width=True,
            )
        with col2:
            st.download_button(
                "📦 Download .zip (with images)",
                buf.getvalue(),
                f"{name}.zip",
                "application/zip",
                use_container_width=True,
            )


if __name__ == "__main__":
    main()
