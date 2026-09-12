# oracle

Black-box dumper for the reference implementation. `tests/oracle/*.raw.txt` were produced with

```
for f in tests/fixtures/*; do
  oracle /path/to/MediaInfo.so raw "$PWD/$f" > tests/oracle/$(basename $f).raw.txt
  oracle /path/to/MediaInfo.so inform "$PWD/$f" > tests/oracle/$(basename $f).txt
done
```

`schema` prints every field (name, measure, options) of each stream kind for a file — the source of
`src/model/schema.rs`. The dumps are data about the reference's *behaviour*; no reference source
code is involved.
