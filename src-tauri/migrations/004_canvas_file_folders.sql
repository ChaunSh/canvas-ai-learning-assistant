-- Canvas folder path for each listed file.
--
-- Canvas organises course files into folders; the Files tab mirrors that layout
-- instead of one flat list. `folder_path` is the folder's full name
-- ("Lecture Slides/Week 1"), empty for files sitting in the course root.

ALTER TABLE canvas_files ADD COLUMN folder_path TEXT;
