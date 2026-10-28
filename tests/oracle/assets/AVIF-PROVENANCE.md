# AVIF fixture provenance

Generated locally with libheif 1.23.4 and AOMedia AV1 encoder 3.15.0.
These tools are independent fixture/reference generators, not renderer dependencies.

```sh
heif-enc --avif --lossless -p chroma=444 -o tests/oracle/assets/image-pattern.avif tests/oracle/assets/image-pattern.png
heif-convert tests/oracle/assets/image-pattern.avif tests/oracle/assets/image-avif-pattern-reference.png
heif-enc --avif --lossless -p chroma=444 --rotate-cw 90 -o tests/oracle/assets/image-rotated.avif tests/oracle/assets/image-pattern.png
heif-convert tests/oracle/assets/image-rotated.avif tests/oracle/assets/image-avif-rotated-reference.png
heif-enc --avif --lossless -p chroma=444 --cut-tiles 64 -o tests/oracle/assets/image-grid.avif tests/oracle/assets/image-pattern.png
heif-convert tests/oracle/assets/image-grid.avif tests/oracle/assets/image-avif-grid-reference.png
heif-enc --avif -q 90 -o tests/oracle/assets/image-lossy.avif tests/oracle/assets/image-pattern.png
heif-convert tests/oracle/assets/image-lossy.avif tests/oracle/assets/image-avif-lossy-reference.png
heif-enc --avif --lossless -b 10 -p chroma=444 -o tests/oracle/assets/image-high-depth.avif tests/oracle/assets/image-high-depth.png
heif-convert tests/oracle/assets/image-high-depth.avif tests/oracle/assets/image-avif-high-depth-reference.png
```

The source images are the existing synthetic HEIF fixtures. The rotated reference
is 40×80; the other references are 80×40. The grid contains two tiles and retains
alpha in this AVIF encoding. The ten-bit reference is stored as sixteen-bit PNG;
sample alignment must be accounted for when comparing decoded integer planes.

The implementation decodes all five; independent pixel tests validate lossless RGB,
alpha and ten-bit precision.

## SHA-256

- `image-avif-grid-reference.png`: `fc25f6053be40192894a28e0b2d65c5998c78f0bb73dfbcb287400d1a9a7588a`
- `image-avif-high-depth-reference.png`: `25c9ece3fb78811761c2277f31bae17ec36ff46f288fc88987dba871997152ef`
- `image-avif-lossy-reference.png`: `e6f45c4cd6c55bb986ef25987d573933f245e570d9008a56e0b8ec74b168fd44`
- `image-avif-pattern-reference.png`: `fc25f6053be40192894a28e0b2d65c5998c78f0bb73dfbcb287400d1a9a7588a`
- `image-avif-rotated-reference.png`: `85bb7c2381ec9b9c397ef4a3b152a6580aa01befb92e0c8b9db4cd61c0c3d683`
- `image-grid.avif`: `db1353c4ad621c7ec02535a973b12ba823a35a7a5bd7030c6c2a70c88d4a8fb9`
- `image-high-depth.avif`: `2dec502b5eb1ba9e4a1e4d54fd46d37f25c406b1951fda4f6fbc7ab1ed5b4b8e`
- `image-lossy.avif`: `2aae410ae2c060ad31e9e2b7226cc7d1c48ec716a289183dd5bebc469f58f944`
- `image-pattern.avif`: `df07c9fb2336298b86abd551bd06a1fc5d1bd34722ed2499ab4a17b8cf65222e`
- `image-rotated.avif`: `fbc8f9abf108bb1a563b3b5aa618a9fa044ec8428e158907f46cb1c4f5635cdd`

## Additional coverage

`python3 tests/oracle/avif_fixtures.py` constructs `image-alpha-grid.avif`
from the existing AV1 grid tiles, using separate colour and alpha grids and idat
item storage. `heif-convert` generates its independent PNG reference. Rust matches
its pixels exactly.

`image-mirrored.avif` was generated with `heif-enc --avif --lossless -p chroma=444
--rotate-cw 90 --flip-h` from `image-pattern.png`. `image-mirror-first.avif`
swaps the primary item's first two ipma associations (0x85, 0x86), leaving the
alpha associations unchanged. Both references were decoded with `heif-convert`.
They exercise independently ordered colour and alpha transforms.

`image-avif-icc-source.png` adds a Pillow ImageCms sRGB profile to the original
pattern. `image-icc.avif` uses lossless 4:4:4 encoding with
`--enable-two-colr-boxes`; its ICC and NCLX property associations are swapped to
put NCLX first. The test requires the original ICC bytes to survive decoding.
The committed ICC source is authoritative because generated profile timestamps
can change between runs.

### Additional SHA-256

- `image-alpha-grid.avif`: `b922cc8f535cbf0d9ab46d5e422b362160d0843bbe354acf9e56d2c2e7dafcf1`
- `image-avif-alpha-grid-reference.png`: `fc25f6053be40192894a28e0b2d65c5998c78f0bb73dfbcb287400d1a9a7588a`
- `image-avif-icc-source.png`: `a55f95293f871f4bea75d1e6129ddfdbbcadc65ac7532558ed55e522bb34c69f`
- `image-icc.avif`: `ddfde3dcd4f088812cced43f6942b2bc84540a617559757b3ac34b4180bc16f7`
- `image-mirrored.avif`: `19a2092ea9574eddf6350c9283ab10cc6980a6de382cf983c72ffe1266b5e265`
- `image-avif-mirrored-reference.png`: `821031577392e02e7910a5fc44c76057f594ecc907d77c3dd09ca1f1fdc0fe07`
- `image-mirror-first.avif`: `aa331fe63a26d060391e9d3ebb05b7124580ed15dafa0cdbe0da3e53b13146ef`
- `image-avif-mirror-first-reference.png`: `be613e87da2cf8b49d31e61cf36d83f5bd20ae8947ac27c6ac0e1e5770e98194`
