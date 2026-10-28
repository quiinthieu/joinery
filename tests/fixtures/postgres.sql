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
