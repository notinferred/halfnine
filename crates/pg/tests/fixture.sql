-- The database the Postgres tests read, and the one Dan's local check opens. Every shape here exists to break something:
-- awkward names, a table without a key, keys in both directions, every common type, NULL beside '', and one huge value.
DROP SCHEMA IF EXISTS tu_fixture CASCADE;
CREATE SCHEMA tu_fixture;
SET search_path = tu_fixture;

CREATE TYPE mood AS ENUM ('calm', 'busy', 'it''s complicated');

CREATE TABLE customers (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  name text NOT NULL,
  email text
);

CREATE TABLE invoices (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  customer_id bigint NOT NULL REFERENCES customers (id),
  amount numeric(12, 2) NOT NULL CHECK (amount >= 0),
  status text NOT NULL DEFAULT 'open',
  due_on date NOT NULL,
  paid_at timestamptz,
  tags text[],
  meta jsonb,
  mood mood,
  ref uuid,
  blob bytea,
  doubled numeric GENERATED ALWAYS AS (amount * 2) STORED,
  note text
);
CREATE INDEX invoices_status_due_on_idx ON invoices (status, due_on);
COMMENT ON COLUMN invoices.note IS 'Free text; NULL and empty are different.';

CREATE TABLE line_items (
  invoice_id bigint REFERENCES invoices (id),
  line int,
  description text,
  PRIMARY KEY (invoice_id, line)
);

CREATE TABLE "odd ""table""" ("weird col" text, "ünïcödé" int);

CREATE TABLE events (id int, at date) PARTITION BY RANGE (at);
CREATE TABLE events_2026 PARTITION OF events FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');

CREATE VIEW open_invoices AS SELECT * FROM invoices WHERE status = 'open';
CREATE MATERIALIZED VIEW totals AS SELECT customer_id, sum(amount) AS total FROM invoices GROUP BY 1;

CREATE FUNCTION add_one(i integer) RETURNS integer LANGUAGE sql IMMUTABLE AS 'SELECT i + 1';
-- Writes when called, so a read-only connection can show that Postgres itself refuses it.
-- The table is named in full because the function runs with the caller's search_path, not this file's.
CREATE FUNCTION touch() RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
  UPDATE tu_fixture.customers SET name = name WHERE id = 1;
  RETURN true;
END
$$;

INSERT INTO customers (name, email)
SELECT 'Customer ' || g, 'customer' || g || '@example.com' FROM generate_series(1, 50) g;

INSERT INTO invoices (customer_id, amount, status, due_on, paid_at, tags, meta, mood, ref, blob, note)
SELECT
  1 + g % 50,
  (g * 3.17)::numeric(12, 2),
  (ARRAY['open', 'paid', 'overdue', 'void'])[1 + g % 4],
  date '2026-01-01' + g % 300,
  CASE WHEN g % 4 = 1 THEN timestamptz '2026-02-01 00:00:00+00' + g * interval '1 minute' END,
  ARRAY['a', 'b,c', 'quote"d'],
  jsonb_build_object('n', g, 'nested', jsonb_build_object('ok', true)),
  (ARRAY['calm', 'busy', 'it''s complicated']::mood[])[1 + g % 3],
  md5(g::text)::uuid,
  decode('deadbeef', 'hex'),
  CASE WHEN g % 10 = 0 THEN NULL WHEN g % 10 = 1 THEN '' ELSE 'note ' || g END
FROM generate_series(1, 120000) g;

INSERT INTO invoices (customer_id, amount, status, due_on, note) VALUES (1, 1, 'open', '2026-01-01', repeat('x', 70000));
INSERT INTO line_items SELECT id, 1, 'line ' || id FROM invoices WHERE id <= 100;
INSERT INTO "odd ""table""" VALUES ('first', 1), (NULL, NULL);

REFRESH MATERIALIZED VIEW totals;
ANALYZE;
