//! Integration tests over the real SQLite schema and the real parsers.
//!
//! These tests deliberately avoid Tauri: they exercise the persistence,
//! extraction, chunking and retrieval layers exactly as the app does, without a
//! GUI harness. Every fixture is generated at test time, so no test data poses as
//! real course content.

use std::path::{Path, PathBuf};

use canvas_ai_assistant_lib::domain::MaterialFilter;
use canvas_ai_assistant_lib::documents::{chunker, extract, import, tokenize};
use canvas_ai_assistant_lib::paths::AppPaths;
use canvas_ai_assistant_lib::retrieval;
use canvas_ai_assistant_lib::retrieval::vector::{SqliteBlobVectorIndex, VectorIndex};
use canvas_ai_assistant_lib::storage::{course_repo, document_repo, Db};

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("caa-it-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn write_file(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent dir");
    }
    std::fs::write(path, contents).expect("write fixture");
}

/// Build a minimal but well-formed DOCX (a ZIP with the required parts).
fn write_docx(path: &Path) {
    use std::io::Write;
    let file = std::fs::File::create(path).expect("create docx");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("[Content_Types].xml", options).unwrap();
    zip.write_all(br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#).unwrap();
    zip.start_file("word/document.xml", options).unwrap();
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
  <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>第一章 导数</w:t></w:r></w:p>
  <w:p><w:r><w:t>导数描述函数在某一点处的瞬时变化率。</w:t></w:r></w:p>
  <w:p><w:r><w:t>定义：若极限存在，则称函数在该点可导。</w:t></w:r></w:p>
  <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>1.1 求导法则</w:t></w:r></w:p>
  <w:p><w:r><w:t>乘积法则与链式法则是计算复合函数导数的基本工具。</w:t></w:r></w:p>
</w:body></w:document>"#;
    zip.write_all(body.as_bytes()).unwrap();
    zip.finish().expect("finish docx");
}

/// Reproduce the indexing steps the pipeline performs, without Tauri.
fn index_material(db: &Db, material_id: &str) {
    let material = db
        .with(|c| document_repo::get(c, material_id))
        .expect("material");
    let extraction = extract::extract(Path::new(&material.storage_path), &material.kind)
        .expect("extraction");
    let chunks = chunker::chunk_pages(&extraction.pages, &chunker::ChunkOptions::default());
    assert!(!chunks.is_empty(), "chunker produced no chunks");

    db.transaction(|tx| {
        let pages: Vec<(i64, String, Option<f64>, Option<f64>, bool)> = extraction
            .pages
            .iter()
            .map(|p| (p.index, p.text.clone(), p.width, p.height, p.has_image))
            .collect();
        document_repo::replace_pages(tx, material_id, &pages)?;

        let seeds: Vec<document_repo::ChunkSeed> = chunks
            .iter()
            .map(|chunk| document_repo::ChunkSeed {
                page_index: Some(chunk.page_index),
                ordinal: chunk.ordinal,
                kind: chunk.kind.as_str().to_string(),
                section: chunk.section.clone(),
                content: chunk.content.clone(),
                context_prefix: chunk.context_prefix.clone(),
                token_estimate: chunk.token_estimate,
                start_char: chunk.start_char,
                end_char: chunk.end_char,
            })
            .collect();
        let ids = document_repo::replace_chunks(tx, material_id, &seeds)?;

        let fts_rows: Vec<retrieval::fts::FtsChunk<'_>> = chunks
            .iter()
            .zip(ids.iter())
            .map(|(chunk, id)| retrieval::fts::FtsChunk {
                chunk_id: id,
                material_id,
                course_id: material.course_id.as_deref(),
                page_index: Some(chunk.page_index),
                index_text: tokenize::index_text(&chunk.content),
            })
            .collect();
        retrieval::fts::replace_material(tx, material_id, &fts_rows)?;
        document_repo::set_index_status(tx, material_id, "indexed", None)?;
        Ok(())
    })
    .expect("index transaction");
}

#[test]
fn end_to_end_markdown_import_and_retrieval() {
    let root = temp_root("e2e");
    let ctx_paths = AppPaths::from_data_dir(root.join("data")).expect("paths");
    let db = Db::open_in_memory().expect("db");

    let course = db
        .with(|c| {
            course_repo::create(
                c,
                canvas_ai_assistant_lib::domain::CreateCourseInput {
                    name: "微积分".into(),
                    code: Some("MATH 101".into()),
                    term: Some("2026 Fall".into()),
                    instructor: Some("Dr. Test".into()),
                    description: None,
                    color: None,
                },
                "manual",
            )
        })
        .expect("course");

    let source = root.join("incoming/lecture-01.md");
    write_file(
        &source,
        "# 第一章 极限\n\n极限描述函数在趋近某点时的趋势。\n\n## 1.2 极限的定义\n\n\
         设函数 f(x) 在 x0 的某去心邻域内有定义。若对任意 ε > 0，存在 δ > 0，\
         使得当 0 < |x - x0| < δ 时，都有 |f(x) - L| < ε，则称 L 为 f 在 x0 处的极限。\n\n\
         ### 例 1\n\n证明 lim_{x→0} sin(x)/x = 1。\n"
            .as_bytes(),
    );

    let imported = import::import_file(
        &db,
        &ctx_paths,
        &source,
        Some(&course.id),
        None,
        "import",
        None,
        None,
    )
    .expect("import");
    assert!(!imported.duplicate);
    assert_eq!(imported.material.kind, "markdown");

    index_material(&db, &imported.material.id);

    let stats = db.with(|c| document_repo::index_stats(c)).expect("stats");
    assert_eq!(stats.0, 1);
    assert_eq!(stats.1, 1);

    let chunks = db
        .with(|c| document_repo::list_chunks(c, &imported.material.id))
        .expect("chunks");
    assert!(chunks.len() >= 2, "expected structural chunks, got {}", chunks.len());
    assert!(
        chunks.iter().any(|chunk| chunk.section.as_deref().unwrap_or("").contains("极限")),
        "a section breadcrumb should be captured"
    );

    // Chinese bigram retrieval must find the definition chunk.
    let index = SqliteBlobVectorIndex::new();
    let outcome = db
        .with(|c| {
            retrieval::search(
                c,
                retrieval::RetrievalRequest {
                    query: "极限的定义",
                    material_ids: &[],
                    course_ids: &[course.id.clone()],
                    limit: 5,
                    query_embedding: None,
                    embedding_model: None,
                },
                &index,
            )
        })
        .expect("search");

    assert!(!outcome.hits.is_empty(), "retrieval returned nothing");
    assert_eq!(outcome.strategy, "fts");
    assert_eq!(outcome.hits[0].material_id, imported.material.id);
    // Markdown has no pagination of its own, so the page is one the app laid
    // out. It must still exist (otherwise the citation cannot jump) and it must
    // be labelled as generated rather than passed off as the file's own page.
    assert!(outcome.hits[0].page.is_some(), "every hit must be jumpable");
    assert_eq!(outcome.hits[0].page_kind.as_deref(), Some("generated"));
    assert!(outcome.hits[0].slide.is_none());
    assert!(
        outcome.hits[0].section.is_some(),
        "section still describes where the excerpt came from"
    );
}

#[test]
fn docx_is_parsed_with_sections_and_searchable() {
    let root = temp_root("docx");
    let paths = AppPaths::from_data_dir(root.join("data")).expect("paths");
    let db = Db::open_in_memory().expect("db");

    let source = root.join("notes.docx");
    write_docx(&source);

    let imported = import::import_file(&db, &paths, &source, None, None, "import", None, None)
        .expect("import docx");
    assert_eq!(imported.material.kind, "docx");
    index_material(&db, &imported.material.id);

    let pages = db
        .with(|c| document_repo::list_pages(c, &imported.material.id))
        .expect("pages");
    let text: String = pages.iter().map(|p| p.text.clone()).collect();
    assert!(text.contains("导数"));
    assert!(text.contains("链式法则") || text.contains("基本工具"));

    let index = SqliteBlobVectorIndex::new();
    let outcome = db
        .with(|c| {
            retrieval::search(
                c,
                retrieval::RetrievalRequest {
                    query: "链式法则",
                    material_ids: &[imported.material.id.clone()],
                    course_ids: &[],
                    limit: 5,
                    query_embedding: None,
                    embedding_model: None,
                },
                &index,
            )
        })
        .expect("search");
    assert!(!outcome.hits.is_empty(), "DOCX content should be retrievable");
    // DOCX pages are app-generated (see `documents::docx::paginate`), so the hit
    // is jumpable and carries provenance instead of pretending to be a PDF.
    assert!(outcome.hits[0].page.is_some());
    assert_eq!(outcome.hits[0].page_kind.as_deref(), Some("generated"));
}

#[test]
fn duplicate_import_is_detected_by_hash() {
    let root = temp_root("dupe");
    let paths = AppPaths::from_data_dir(root.join("data")).expect("paths");
    let db = Db::open_in_memory().expect("db");

    let a = root.join("a/lecture.pdf");
    let b = root.join("b/copy-of-lecture.pdf");
    let bytes = b"%PDF-1.4\n% fixture\n";
    write_file(&a, bytes);
    write_file(&b, bytes);

    let first = import::import_file(&db, &paths, &a, None, None, "import", None, None).unwrap();
    let second = import::import_file(&db, &paths, &b, None, None, "import", None, None).unwrap();

    assert!(!first.duplicate);
    assert!(second.duplicate, "identical bytes must be de-duplicated");
    assert_eq!(first.material.sha256, second.material.sha256);
    assert_eq!(first.material.id, second.material.id);
}

#[test]
fn zip_import_expands_and_rejects_traversal() {
    use std::io::Write;
    let root = temp_root("zip");
    let paths = AppPaths::from_data_dir(root.join("data")).expect("paths");
    let db = Db::open_in_memory().expect("db");

    let archive_path = root.join("bundle.zip");
    {
        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("week1/notes.md", options).unwrap();
        zip.write_all(b"# Week 1\n\ncontent\n").unwrap();
        zip.start_file("../escape.md", options).unwrap();
        zip.write_all(b"# nope\n").unwrap();
        zip.finish().unwrap();
    }

    let (imported, skipped) =
        import::import_zip(&db, &paths, &archive_path, None, None, "import").expect("zip import");
    assert_eq!(imported.len(), 1, "only the safe entry should import");
    assert!(!root.join("escape.md").exists(), "traversal must be blocked");
    assert!(
        skipped.is_empty() || skipped.iter().all(|s| !s.contains("escape.md")),
        "unexpected skip report: {skipped:?}"
    );
}

#[test]
fn database_backup_produces_a_reopenable_snapshot() {
    let root = temp_root("backup");
    let db_path = root.join("data/canvas-ai.db");
    let db = Db::open(&db_path).expect("open db");

    db.with(|c| {
        course_repo::create(
            c,
            canvas_ai_assistant_lib::domain::CreateCourseInput {
                name: "线性代数".into(),
                code: None,
                term: None,
                instructor: None,
                description: None,
                color: None,
            },
            "manual",
        )
    })
    .expect("seed course");

    let backup = root.join("backups/snapshot.sqlite");
    let size = db.backup_to(&backup).expect("backup");
    assert!(size > 0);

    let restored = Db::open(&backup).expect("reopen backup");
    let courses = restored
        .with(|c| course_repo::list(c, true))
        .expect("list restored");
    assert_eq!(courses.len(), 1);
    assert_eq!(courses[0].name, "线性代数");
}

#[test]
fn calendar_ics_roundtrip_keeps_utc_instants() {
    let db = Db::open_in_memory().expect("db");
    let event = db
        .with(|c| {
            canvas_ai_assistant_lib::storage::coursework_repo::create_event(
                c,
                canvas_ai_assistant_lib::domain::CreateEventInput {
                    course_id: None,
                    title: "期中考试".into(),
                    description: Some("第一至第四章".into()),
                    location: Some("Room 2405".into()),
                    starts_at: "2026-11-01T02:00:00Z".into(),
                    ends_at: Some("2026-11-01T04:00:00Z".into()),
                    all_day: false,
                    kind: Some("exam".into()),
                },
            )
        })
        .expect("create event");

    let rendered = canvas_ai_assistant_lib::documents::ics::render(&[event]);
    assert!(rendered.contains("BEGIN:VEVENT"));
    assert!(rendered.contains("DTSTART:20261101T020000Z"));

    let parsed = canvas_ai_assistant_lib::documents::ics::parse(&rendered).expect("parse");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].starts_at, "2026-11-01T02:00:00Z");
    assert_eq!(parsed[0].summary, "期中考试");
}

#[test]
fn real_pdf_reports_true_page_numbers() {
    // Fixtures are real LaTeX output committed under `fixtures/pdf`:
    //  - sample-zh.pdf: xeCJK with ToUnicode CMaps (the good path)
    //  - sample-en.pdf: pdfTeX Type1 fonts (no ToUnicode; fallback encoding path)
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/pdf");

    let zh = extract::extract(&base.join("sample-zh.pdf"), "pdf").expect("extract zh pdf");
    assert_eq!(zh.pages.len(), 2, "expected two real pages");
    assert_eq!(zh.pages[0].index, 0);
    assert_eq!(zh.pages[1].index, 1);
    assert!(
        zh.pages[0].text.contains("导数的定义"),
        "page 1 should contain the first section heading"
    );
    assert!(
        zh.pages[0].text.contains("可导"),
        "page 1 should contain body text about differentiability"
    );
    assert!(
        zh.pages[1].text.contains("链式法则"),
        "page 2 should contain the second section heading"
    );
    assert!(
        !zh.pages[0].text.contains("链式法则"),
        "sections must not leak across real page boundaries"
    );

    let en = extract::extract(&base.join("sample-en.pdf"), "pdf").expect("extract en pdf");
    assert_eq!(en.pages.len(), 2);
    assert!(en.pages[0].text.contains("Derivatives"));
    assert!(en.pages[1].text.contains("Integration"));
    assert!(
        !en.pages[0].text.contains("Integration"),
        "page 1 must not contain page 2's heading"
    );
}

#[test]
fn material_filtering_and_deletion_cleanup() {
    let root = temp_root("lifecycle");
    let paths = AppPaths::from_data_dir(root.join("data")).expect("paths");
    let db = Db::open_in_memory().expect("db");

    let source = root.join("doc.md");
    write_file(&source, b"# Title\n\nbody\n");
    let imported = import::import_file(&db, &paths, &source, None, None, "import", None, None)
        .expect("import");
    let blob = imported.material.storage_path.clone();
    assert!(Path::new(&blob).exists());

    index_material(&db, &imported.material.id);
    let all = db
        .with(|c| {
            document_repo::list(
                c,
                &MaterialFilter {
                    course_id: None,
                    kind: None,
                    query: None,
                    favorites_only: false,
                    index_status: None,
                    limit: None,
                    offset: None,
                },
            )
        })
        .expect("list");
    assert_eq!(all.len(), 1);
    assert!(all[0].chunk_count > 0);

    let index = SqliteBlobVectorIndex::new();
    db.transaction(|tx| {
        retrieval::fts::delete_material(tx, &imported.material.id)?;
        index.delete_material(tx, &imported.material.id)?;
        document_repo::delete(tx, &imported.material.id)?;
        Ok(())
    })
    .expect("delete");

    let remaining = db
        .with(|c| {
            document_repo::list(
                c,
                &MaterialFilter {
                    course_id: None,
                    kind: None,
                    query: None,
                    favorites_only: false,
                    index_status: None,
                    limit: None,
                    offset: None,
                },
            )
        })
        .expect("list after delete");
    assert!(remaining.is_empty());
    // The blob is only removed by the command layer, which owns the filesystem.
    assert!(Path::new(&blob).exists());
}
