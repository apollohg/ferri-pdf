# Local changes to ICU4X icu_casemap 2.3.0

Upstream: https://github.com/unicode-org/icu4x (Unicode-3.0; LICENSE retained).

The renderer needs complete source context while assigning each mapped scalar
back to its CSS style, including expansions and removals across inline boundaries.
Upstream's string-only API does not expose these source boundaries.

- Expose a forward `CaseMappingContext` cursor that writes one scalar through the
  existing full Unicode case-mapping helper. Offsets must be nondecreasing and
  valid UTF-8 boundaries. Mapping still uses the original source on both sides.
- Reuse the Dutch IJ prefix detector for CSS capitalization.
- Accumulate preceding Greek-letter state in the cursor, avoiding quadratic
  backward scans when removing long runs of combining accents. The original
  string API retains its existing behavior.

The document crate chooses CSS word/letter units, preserves existing uppercase
letters during capitalization, and limits expanded output before appending.
No Unicode data tables or case mappings have been replaced.
