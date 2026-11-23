#!/usr/bin/env bash
# Copy the staged extensions into a scratch copy of the server, start it, and run a
# hybrid (BM25 + vector) query end to end. Needs $PG (server dir) and $OUT (staged files).
set -euo pipefail
: "${PG:?}" "${OUT:?}"
work=$(mktemp -d)
cp -a "$PG" "$work/pg"
cp -a "$OUT/lib/." "$work/pg/lib/"
cp -a "$OUT/share/extension/." "$work/pg/share/extension/"
bin="$work/pg/bin"
"$bin/initdb" -D "$work/data" -U postgres -A trust > /dev/null
trap '"$bin/pg_ctl" -D "$work/data" -m fast stop > /dev/null 2>&1 || true; echo "--- server log"; tail -40 "$work/log" 2>/dev/null || true' EXIT
"$bin/pg_ctl" -D "$work/data" -o "-p 55432 -c listen_addresses=localhost -c shared_preload_libraries=pg_tokenizer,vchord_bm25" -l "$work/log" -w start
psql() { "$bin/psql" -h localhost -p 55432 -U postgres -d postgres -v ON_ERROR_STOP=1 -qAt "$@"; }
psql <<'SQL'
CREATE EXTENSION vector;
CREATE EXTENSION pg_tokenizer CASCADE;
CREATE EXTENSION vchord_bm25 CASCADE;
SET search_path TO public, tokenizer_catalog, bm25_catalog;
SELECT create_tokenizer('bert', $$ model = "bert_base_uncased" $$);
CREATE TABLE docs (id int PRIMARY KEY, body text, bm25 bm25vector, emb vector(3));
INSERT INTO docs (id, body, emb) VALUES
  (1, 'Erb palsy is an upper trunk brachial plexus injury at C5 and C6', '[1,0,0]'),
  (2, 'Klumpke palsy affects the lower trunk, C8 and T1', '[0,1,0]'),
  (3, 'The femoral nerve supplies the quadriceps', '[0,0,1]');
UPDATE docs SET bm25 = tokenize(body, 'bert');
CREATE INDEX docs_bm25 ON docs USING bm25 (bm25 bm25_ops);
CREATE INDEX docs_emb ON docs USING hnsw (emb vector_cosine_ops);
SQL
top=$(psql -c "SET search_path TO public, tokenizer_catalog, bm25_catalog;" -c "
WITH b AS (SELECT id, row_number() OVER (ORDER BY bm25 <&> to_bm25query('docs_bm25', tokenize('upper trunk C5', 'bert'))) r
           FROM docs ORDER BY bm25 <&> to_bm25query('docs_bm25', tokenize('upper trunk C5', 'bert')) LIMIT 2),
     v AS (SELECT id, row_number() OVER (ORDER BY emb <=> '[0.9,0.1,0]') r FROM docs ORDER BY emb <=> '[0.9,0.1,0]' LIMIT 2)
SELECT coalesce(b.id, v.id) FROM b FULL JOIN v USING (id)
ORDER BY coalesce(1.0/(60+b.r), 0) + coalesce(1.0/(60+v.r), 0) DESC LIMIT 1;")
echo "hybrid top hit: $top"
[ "$top" = 1 ]
