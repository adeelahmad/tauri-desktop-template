#!/usr/bin/env bash
# Build one pgrx extension against the embedded Postgres and stage it under $OUT
# (lib/ and share/extension/, the layout of a postgresql_embedded install).
# Usage: build-rust-ext.sh <source dir> <extension name> <semver>
set -euo pipefail
src=$1 name=$2 semver=$3
: "${PG_CONFIG:?}" "${OUT:?}"
cd "$src"
cargo build --lib --features pg16 --release
cargo pgrx schema --features pg16 --pg-config "$PG_CONFIG" --out "target/$name--$semver.sql"
mkdir -p "$OUT/lib" "$OUT/share/extension"
[ -d sql/upgrade ] && cp -a sql/upgrade/. "$OUT/share/extension/"
ls sql/*.sql >/dev/null 2>&1 && cp sql/*.sql "$OUT/share/extension/"
cp "target/$name--$semver.sql" "$OUT/share/extension/"
sed -e "s/@CARGO_VERSION@/$semver/g" "$name.control" > "$OUT/share/extension/$name.control"
case "$(uname -s)" in
  Darwin) cp "target/release/lib$name.dylib" "$OUT/lib/$name.dylib" ;;
  MINGW*|MSYS*|CYGWIN*) cp "target/release/$name.dll" "$OUT/lib/$name.dll" ;;
  *) cp "target/release/lib$name.so" "$OUT/lib/$name.so" ;;
esac
