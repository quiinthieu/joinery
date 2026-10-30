SET NAMES utf8mb4;
CREATE TABLE sample (
  id BIGINT UNSIGNED PRIMARY KEY,
  amount DECIMAL(40,9) NOT NULL,
  note TEXT NULL,
  payload JSON NULL,
  raw_value VARBINARY(16) NULL,
  recorded_at DATETIME(6) NULL,
  generated_value BIGINT UNSIGNED GENERATED ALWAYS AS (id + 1) STORED
) ENGINE=InnoDB;
INSERT INTO sample (id,amount,note,payload,raw_value,recorded_at)
SELECT 9007199254740992+n,12345678901234567890.123456789,
       CASE WHEN n=0 THEN NULL WHEN n=1 THEN '' ELSE CONCAT('row ',n) END,
       '{"fixture":true}',X'00FF', '2026-01-02 03:04:05.123456'
FROM (
  SELECT a.n+10*b.n+100*c.n AS n FROM
  (SELECT 0 n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) a
  CROSS JOIN (SELECT 0 n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) b
  CROSS JOIN (SELECT 0 n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) c
) numbers;
CREATE TABLE composite_keys (part BIGINT NOT NULL, name VARCHAR(32) NOT NULL, value VARCHAR(100), PRIMARY KEY(name,part)) ENGINE=InnoDB;
INSERT INTO composite_keys VALUES (2,'a','two'),(10,'a','ten'),(1,'b','one');
CREATE TABLE child_rows (id BIGINT PRIMARY KEY, parent_id BIGINT UNSIGNED, FOREIGN KEY(parent_id) REFERENCES sample(id)) ENGINE=InnoDB;
CREATE TABLE nontransactional (id INT PRIMARY KEY, value TEXT) ENGINE=MyISAM;
INSERT INTO nontransactional VALUES (1,'read-only grid');
CREATE VIEW sample_view AS SELECT id,note FROM sample;

CREATE TABLE `completion names` (`No``tes` TEXT NOT NULL, `列` INTEGER NOT NULL) ENGINE=InnoDB;
INSERT INTO `completion names` VALUES ('completion fixture', 42);

-- Disposable live completion context fixtures.
CREATE DATABASE completion_alt;
CREATE TABLE completion_alt.sample (alt_note TEXT);
INSERT INTO completion_alt.sample VALUES ('alternate context');
CREATE TABLE completion_alt.column_grants (visible_note TEXT, private_note TEXT);
CREATE TABLE completion_alt.`CaseOnly` (case_note TEXT);
CREATE ROLE completion_reader;
GRANT SELECT ON completion_alt.sample TO completion_reader;
GRANT SELECT ON completion_alt.`CaseOnly` TO completion_reader;
GRANT SELECT (visible_note), UPDATE (private_note) ON completion_alt.column_grants TO completion_reader;
GRANT completion_reader TO 'joinery'@'%';

-- Grouped foreign-key fixtures; source names differ from targets to prove
-- ordinal pairing, and note_order participates in two constraints.
CREATE TABLE note_orders (id BIGINT PRIMARY KEY) ENGINE=InnoDB;
INSERT INTO note_orders VALUES (7),(8);
CREATE TABLE order_lines (
  order_id BIGINT NOT NULL,
  line_no INT NOT NULL,
  sku VARCHAR(40) NOT NULL,
  PRIMARY KEY (order_id, line_no),
  FOREIGN KEY (order_id) REFERENCES note_orders(id)
) ENGINE=InnoDB;
INSERT INTO order_lines VALUES (7,1,'SKU-7-1'),(7,2,'SKU-7-2'),(8,1,'SKU-8-1');
CREATE TABLE line_notes (
  id BIGINT PRIMARY KEY,
  note_order BIGINT NOT NULL,
  note_line INT NULL,
  note VARCHAR(100) NOT NULL,
  CONSTRAINT line_notes_line_fk FOREIGN KEY (note_order, note_line) REFERENCES order_lines(order_id, line_no),
  CONSTRAINT line_notes_order_fk FOREIGN KEY (note_order) REFERENCES note_orders(id)
) ENGINE=InnoDB;
INSERT INTO line_notes VALUES (1,7,2,'composite target'),(2,7,NULL,'line unknown');

-- Sorted/filtered table-browsing fixture: NULLs, duplicates, mixed case and
-- LIKE metacharacters across three keyset pages.
CREATE TABLE browse_sort (id BIGINT PRIMARY KEY, bucket INT NULL, label VARCHAR(40) NULL) ENGINE=InnoDB;
INSERT INTO browse_sort (id,bucket,label)
SELECT n, IF(n % 7 = 0, NULL, n % 10),
       CASE WHEN n % 11 = 0 THEN NULL
            WHEN n % 13 = 0 THEN CONCAT('odd_%', n)
            ELSE ELT(1 + n % 3, 'alpha', 'Beta', 'gamma') END
FROM (
  SELECT a.n+10*b.n+100*c.n+1000*d.n+1 AS n FROM
  (SELECT 0 n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) a
  CROSS JOIN (SELECT 0 n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) b
  CROSS JOIN (SELECT 0 n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) c
  CROSS JOIN (SELECT 0 n UNION ALL SELECT 1) d
) numbers WHERE n <= 1500;
