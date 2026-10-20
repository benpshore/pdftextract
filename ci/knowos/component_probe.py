"""Public-dependency smoke probe; contains no private documents or credentials."""
import os
os.environ['ORT_DISABLE_TELEMETRY'] = '1'
os.environ['HF_HUB_DISABLE_TELEMETRY'] = '1'
os.environ['HF_HUB_DISABLE_IMPLICIT_TOKEN'] = '1'
os.environ['DO_NOT_TRACK'] = '1'
import json
import tempfile
from pathlib import Path
from reportlab.pdfgen.canvas import Canvas
import pdf_oxide
from docling.datamodel.base_models import InputFormat
from docling.datamodel.pipeline_options import PdfPipelineOptions
from docling.document_converter import DocumentConverter, PdfFormatOption

with tempfile.TemporaryDirectory() as folder:
    path = Path(folder) / 'synthetic.pdf'
    canvas = Canvas(str(path))
    canvas.drawString(60, 720, 'NEURAL-42: The sample contains 17 cells.')
    canvas.save()
    doc = pdf_oxide.PdfDocument(str(path))
    assert '17 cells' in doc.extract_text(0)
    options = PdfPipelineOptions()
    options.do_ocr = True
    options.do_table_structure = True
    options.enable_remote_services = False
    full = os.environ.get('PROBE_FULL') == '1'
    options.do_code_enrichment = full
    options.do_formula_enrichment = full
    options.do_picture_classification = full
    options.do_picture_description = full
    result = DocumentConverter(format_options={InputFormat.PDF: PdfFormatOption(pipeline_options=options)}).convert(path)
    assert '17 cells' in result.document.export_to_text()
    print(json.dumps({'pdf_oxide': 'pass', 'docling': 'pass', 'full': full}))
