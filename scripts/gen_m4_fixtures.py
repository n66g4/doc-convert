#!/usr/bin/env python3
"""
生成 M4 固定样本（SRS §1.8 Must 输入类型），写入 tests/fixtures/m4/。
仅使用标准库；在仓库根执行: python3 scripts/gen_m4_fixtures.py
"""
from __future__ import annotations

import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests" / "fixtures" / "m4"


def build_pdf_v2() -> bytes:
    """单遍写入并生成 xref（一页文本型 PDF，满足 Must「文本型 PDF」样本）。"""
    stream_inner = b"BT /F1 12 Tf 72 720 Td (M4 PDF text sample) Tj ET\n"
    stream_body = (
        b"<< /Length "
        + str(len(stream_inner)).encode()
        + b" >>\nstream\n"
        + stream_inner
        + b"endstream"
    )
    bodies = [
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Font << /F1 1 0 R >> >>",
        stream_body,
        b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 612 792] "
        b"/Contents 3 0 R /Resources 2 0 R >>",
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
        b"<< /Type /Catalog /Pages 5 0 R >>",
    ]
    header = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n"
    parts: list[bytes] = [header]
    obj_offsets: list[int] = []
    for i, body in enumerate(bodies, start=1):
        obj_offsets.append(sum(len(x) for x in parts))
        parts.append(f"{i} 0 obj\n".encode() + body + b"\nendobj\n")
    xref_start = sum(len(x) for x in parts)
    parts.append(f"xref\n0 {len(bodies)+1}\n".encode())
    parts.append(b"0000000000 65535 f \n")
    for off in obj_offsets:
        parts.append(f"{off:010d} 00000 n \n".encode())
    parts.append(
        b"trailer\n<< /Size "
        + str(len(bodies) + 1).encode()
        + b" /Root 6 0 R >>\nstartxref\n"
        + str(xref_start).encode()
        + b"\n%%EOF\n"
    )
    return b"".join(parts)


def write_docx(path: Path) -> None:
    ct = """<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"""
    rels = """<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"""
    wrels = """<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"""
    doc = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body><w:p><w:r><w:t>M4 Word sample</w:t></w:r></w:p></w:body>
</w:document>"""
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", ct)
        z.writestr("_rels/.rels", rels)
        z.writestr("word/document.xml", doc)
        z.writestr("word/_rels/document.xml.rels", wrels)


def write_xlsx(path: Path) -> None:
    ct = """<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"""
    rels = """<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"""
    wb = """<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"""
    wb_rels = """<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"""
    sheet = """<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>M4 Excel sample</t></is></c></row>
  </sheetData>
</worksheet>"""
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", ct)
        z.writestr("_rels/.rels", rels)
        z.writestr("xl/workbook.xml", wb)
        z.writestr("xl/_rels/workbook.xml.rels", wb_rels)
        z.writestr("xl/worksheets/sheet1.xml", sheet)


def write_pptx(path: Path) -> None:
    ct = """<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"""
    rels = """<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"""
    pres = """<?xml version="1.0" encoding="UTF-8"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
</p:presentation>"""
    pres_rels = """<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"""
    slide = """<?xml version="1.0" encoding="UTF-8"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp>
    <p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
    <p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>M4 PowerPoint sample</a:t></a:r></a:p></p:txBody>
  </p:sp></p:spTree></p:cSld>
</p:sld>"""
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", ct)
        z.writestr("_rels/.rels", rels)
        z.writestr("ppt/presentation.xml", pres)
        z.writestr("ppt/_rels/presentation.xml.rels", pres_rels)
        z.writestr("ppt/slides/slide1.xml", slide)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "must_plain.txt").write_text("M4 plain text sample.\nSecond line.\n", encoding="utf-8")
    (OUT / "must_minimal.rtf").write_text(
        r"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}}\f0\fs24 M4 RTF sample.\par}",
        encoding="ascii",
    )
    (OUT / "must_simple.html").write_text(
        "<!DOCTYPE html><html><head><meta charset='utf-8'><title>M4</title></head>"
        "<body><h1>M4 HTML</h1><p>Paragraph.</p></body></html>\n",
        encoding="utf-8",
    )
    write_docx(OUT / "must_word.docx")
    write_xlsx(OUT / "must_excel.xlsx")
    write_pptx(OUT / "must_slides.pptx")
    (OUT / "must_pdf_text.pdf").write_bytes(build_pdf_v2())
    print("Wrote fixtures to", OUT)


if __name__ == "__main__":
    main()
