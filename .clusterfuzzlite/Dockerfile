FROM gcr.io/oss-fuzz-base/base-builder-rust
COPY . $SRC/sqlparser-plpgsql
WORKDIR $SRC/sqlparser-plpgsql
COPY ./.clusterfuzzlite/build.sh $SRC/
