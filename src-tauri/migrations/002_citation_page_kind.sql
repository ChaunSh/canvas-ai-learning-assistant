-- Page provenance for citations.
--
-- Until now only PDFs and slide decks carried a page number; DOCX/text/markdown
-- citations fell back to a section name, so a jump was impossible. Every chunk
-- now gets a page, and this column records where it came from:
--
--   native    -> the document's own pagination (PDF)
--   slide     -> a slide number (PPTX)
--   generated -> the app laid out pages for a format that has none
--
-- The UI labels `generated` explicitly, so a synthetic page is never presented
-- as if it were the file's own page number.

ALTER TABLE chat_citations ADD COLUMN page_kind TEXT;
