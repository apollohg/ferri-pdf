# Emoji presentation data

`variation_bases.rs` is generated from the Unicode 17.0.0 [emoji-variation-sequences.txt](https://www.unicode.org/Public/17.0.0/ucd/emoji/emoji-variation-sequences.txt), SHA-256 `bb3d09ef03f206012c7532dd52dc0a21c9efddba0135ea4cf0d9201b8b9bba7e`. The file's terms are the Unicode-3.0 licence in `LICENSE`.

Regenerate from the downloaded file:

```sh
python3 spec/emoji/generate.py tmp/emoji-variation-sequences-17.0.0.txt
```

The generator rejects a different file hash. The table lists the 371 code points that have text and emoji presentation sequences, which CSS Fonts 4 §9.3 calls Emoji Presentation Participating Code Points. `font-variant-emoji` appends a presentation selector only to these code points. Rendering performs no Unicode data download.
