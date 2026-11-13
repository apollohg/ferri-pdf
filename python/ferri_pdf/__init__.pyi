import os
from collections.abc import Callable, Iterable, Sequence
from typing import Any, Literal

__version__: str

class RenderError(Exception): ...

class ContentLostError(RenderError):
    diagnostics: tuple[Diagnostic, ...]
    part: int | None

class RenderOptions:
    """Settings for a render.

    Local files are read only inside the folder `base` or a `file:` `base_url` names, and
    never for content with a remote base; without a base none is read. `file_root` names
    another folder to read inside instead.
    """

    def __init__(
        self,
        *,
        base: str | os.PathLike[str] | None = None,
        base_url: str | None = None,
        stylesheets: Sequence[str] = (),
        user_stylesheets: Sequence[str] = (),
        paper: tuple[float, float] | None = None,
        color_scheme: Literal["light", "dark"] = "light",
        contrast: Literal["no-preference", "less", "more", "custom"] = "no-preference",
        reduced_motion: Literal["no-preference", "reduce"] = "no-preference",
        serif: Sequence[str] = (),
        sans_serif: Sequence[str] = (),
        monospace: Sequence[str] = (),
        fallback: Sequence[str] = (),
        system_fonts: bool = False,
        font_dirs: Sequence[str | os.PathLike[str]] = (),
        image_dpi: float | None = None,
        jpeg_quality: int | None = None,
        include_layout: bool = False,
        allow_content_loss: bool = False,
        allow_http: bool = True,
        file_root: str | os.PathLike[str] | None = None,
        fetch: Callable[[str], bytes | Resource | None] | None = None,
    ) -> None: ...

class Resource:
    def __init__(
        self, data: bytes, *, content_type: str | None = None, url: str | None = None
    ) -> None: ...

class Part:
    def __init__(
        self,
        html: str | bytes,
        *,
        base: str | os.PathLike[str] | None = None,
        base_url: str | None = None,
    ) -> None: ...

class Diagnostic:
    @property
    def message(self) -> str: ...
    @property
    def loses_content(self) -> bool: ...

class Heading:
    @property
    def id(self) -> str | None: ...
    @property
    def level(self) -> int: ...
    @property
    def text(self) -> str: ...
    @property
    def page(self) -> int: ...
    @property
    def top(self) -> float: ...

class RenderOutput:
    @property
    def pdf(self) -> bytes: ...
    @property
    def page_count(self) -> int: ...
    @property
    def headings(self) -> tuple[Heading, ...]: ...
    @property
    def diagnostics(self) -> tuple[Diagnostic, ...]: ...
    @property
    def layout(self) -> Any | None: ...

class PartOutput:
    @property
    def first_page(self) -> int: ...
    @property
    def page_count(self) -> int: ...
    @property
    def headings(self) -> tuple[Heading, ...]: ...
    @property
    def diagnostics(self) -> tuple[Diagnostic, ...]: ...
    @property
    def layout(self) -> Any | None: ...

class MultipartOutput:
    @property
    def pdf(self) -> bytes: ...
    @property
    def page_count(self) -> int: ...
    @property
    def parts(self) -> tuple[PartOutput, ...]: ...

def render(html: str | bytes, options: RenderOptions | None = None) -> RenderOutput: ...
def render_parts(
    parts: Iterable[str | bytes | Part], options: RenderOptions | None = None
) -> MultipartOutput: ...
