# Quotation data

`quotes.rs` is generated from [Unicode CLDR 48 core.zip](https://www.unicode.org/Public/cldr/48/core.zip), SHA-256 `06c7c698d6fd8d67cefac15a0206b0109b00e0ef1636f86f84449fa959561f74`. The archive's Unicode-3.0 licence is retained in `LICENSE`.

Regenerate from the downloaded archive:

```sh
python3 spec/cldr/generate.py tmp/cldr-48-core.zip
```

The generator rejects a different archive hash. It resolves the four delimiter fields through CLDR's explicit parent locales and ordinary locale inheritance, then adds likely-subtag and language-alias lookup keys. Identical systems share an entry. The result contains 9,071 locale keys and 28 distinct systems. Rendering uses static data and performs no locale-data download or host-locale lookup.

The UA uses these delimiter pairs for `quotes:auto`, as permitted by [CSS Generated Content 3](https://www.w3.org/TR/2025/WD-css-content-3-20251204/#quotes-property). Unknown languages use CLDR's root system. Lookup is case insensitive, ignores locale extensions, and retains regional choices when an optional script subtag is present. The final delimiter pair repeats at greater quote depths, following CSS rather than cycling back to the first pair.
