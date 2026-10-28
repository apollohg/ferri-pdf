# STIX metric fixture

`STIXGeneral.ttf` is a test asset, not a bundled runtime fallback. It is the 1000-unit TrueType face distributed in [Matplotlib v3.10.8](https://github.com/matplotlib/matplotlib/blob/v3.10.8/lib/matplotlib/mpl-data/fonts/ttf/STIXGeneral.ttf). Its native 511-unit arrow advances expose premature float rounding in PDF font-width conversion. Fixtures load this explicit file rather than a system font.

SHA-256: `167378031e2dddc6216d67819c9260e9a06ffc4c478e4e23cb98a6fd44b183c2`.

`STIX-LICENSE.txt` preserves the [upstream licence and conversion notice](https://github.com/matplotlib/matplotlib/blob/v3.10.8/lib/matplotlib/mpl-data/fonts/ttf/LICENSE_STIX), SHA-256 `bab3d31dfef07f483624f2f65f2711e76065b8e7273278b1c071ede1041c9959`.
