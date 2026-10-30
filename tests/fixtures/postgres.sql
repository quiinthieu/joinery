CREATE TABLE customers (
  id bigint PRIMARY KEY,
  name text NOT NULL,
  email text,
  active boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  metadata jsonb
);
CREATE TABLE orders (
  id bigint PRIMARY KEY,
  customer_id bigint NOT NULL REFERENCES customers(id),
  status text NOT NULL,
  total numeric(30,8) NOT NULL,
  placed_at timestamptz NOT NULL DEFAULT now(),
  notes text
);
INSERT INTO customers(id,name,email,active,metadata)
SELECT i, 'Customer ' || i, 'customer' || i || '@example.test', i % 7 <> 0,
       jsonb_build_object('tier', CASE WHEN i % 3 = 0 THEN 'pro' ELSE 'starter' END)
FROM generate_series(1,1000) AS i;
INSERT INTO orders(id,customer_id,status,total,notes)
SELECT i, (i % 1000)+1,
       (ARRAY['completed','processing','pending','refunded'])[1+(i%4)],
       (i * 12.349)::numeric(30,8),
       CASE WHEN i % 9 = 0 THEN NULL ELSE 'Order ' || i END
FROM generate_series(1,10000) AS i;
ANALYZE;

CREATE TABLE "completion.names" ("No""tes" text NOT NULL, "列" integer NOT NULL);
INSERT INTO "completion.names" VALUES ('completion fixture', 42);

-- Disposable live-context fixtures; no runtime test needs destructive DDL.
CREATE SCHEMA completion_alt;
CREATE TABLE completion_alt.orders (alt_note text);
INSERT INTO completion_alt.orders VALUES ('alternate context');
CREATE ROLE joinery_completion_reader NOLOGIN;
GRANT joinery_completion_reader TO joinery;
GRANT USAGE ON SCHEMA completion_alt TO joinery_completion_reader;
GRANT SELECT ON completion_alt.orders TO joinery_completion_reader;
CREATE TABLE completion_alt.column_grants (visible_note text, private_note text);
GRANT SELECT (visible_note) ON completion_alt.column_grants TO joinery_completion_reader;

-- Grouped foreign-key fixtures; source names differ from targets to prove
-- ordinal pairing, and note_order participates in two constraints.
CREATE TABLE order_lines (
  order_id bigint NOT NULL REFERENCES orders(id),
  line_no integer NOT NULL,
  sku text NOT NULL,
  PRIMARY KEY (order_id, line_no)
);
INSERT INTO order_lines
SELECT o, l, 'SKU-' || o || '-' || l FROM generate_series(1,20) AS o, generate_series(1,3) AS l;
CREATE TABLE line_notes (
  id bigint PRIMARY KEY,
  note_order bigint NOT NULL,
  note_line integer,
  note text NOT NULL,
  CONSTRAINT line_notes_line_fk FOREIGN KEY (note_order, note_line) REFERENCES order_lines(order_id, line_no),
  CONSTRAINT line_notes_order_fk FOREIGN KEY (note_order) REFERENCES orders(id)
);
INSERT INTO line_notes VALUES (1, 7, 2, 'composite target'), (2, 7, NULL, 'line unknown');

-- Sorted/filtered table-browsing fixture: NULLs, duplicates, mixed case and
-- LIKE metacharacters across three keyset pages.
CREATE TABLE browse_sort (id bigint PRIMARY KEY, bucket integer, label text);
INSERT INTO browse_sort
SELECT i,
       CASE WHEN i % 7 = 0 THEN NULL ELSE i % 10 END,
       CASE WHEN i % 11 = 0 THEN NULL
            WHEN i % 13 = 0 THEN 'odd_%' || i
            ELSE (ARRAY['alpha','Beta','gamma'])[1 + i % 3] END
FROM generate_series(1,1500) AS i;
