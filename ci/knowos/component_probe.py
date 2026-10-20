"""Public-dependency smoke probe; contains no private documents or credentials."""

import os

# Set before importing docling/pdf_oxide so telemetry and implicit auth stay off.
os.environ.update(
    dict.fromkeys(
        (
            "ORT_DISABLE_TELEMETRY",
            "HF_HUB_DISABLE_TELEMETRY",
            "HF_HUB_DISABLE_IMPLICIT_TOKEN",
            "DO_NOT_TRACK",
        ),
        "1",
    )
)
import json
import tempfile
from pathlib import Path

import pdf_oxide
from docling.datamodel.base_models import InputFormat
from docling.datamodel.pipeline_options import PdfPipelineOptions
from docling.document_converter import DocumentConverter, PdfFormatOption
from reportlab.pdfgen.canvas import Canvas

with tempfile.TemporaryDirectory() as folder:
    path = Path(folder) / "synthetic.pdf"
    canvas = Canvas(str(path))
    canvas.drawString(60, 720, "NEURAL-42: The sample contains 17 cells.")
    canvas.save()
    doc = pdf_oxide.PdfDocument(str(path))
    if "17 cells" not in doc.extract_text(0):
        raise SystemExit("pdf_oxide probe failed: expected text not extracted")
    options = PdfPipelineOptions()
    options.do_ocr = True
    options.do_table_structure = True
    options.enable_remote_services = False
    full = os.environ.get("PROBE_FULL") == "1"
    options.do_code_enrichment = full
    options.do_formula_enrichment = full
    options.do_picture_classification = full
    options.do_picture_description = full
    result = DocumentConverter(
        format_options={InputFormat.PDF: PdfFormatOption(pipeline_options=options)}
    ).convert(path)
    if "17 cells" not in result.document.export_to_text():
        raise SystemExit("docling probe failed: expected text not extracted")
    print(json.dumps({"pdf_oxide": "pass", "docling": "pass", "full": full}))
