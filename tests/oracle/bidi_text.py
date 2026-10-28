"""Independent Unicode bidi ordering for PDF replacement-text corroboration."""
import ctypes
import ctypes.util
from functools import lru_cache
import os
import sys


class BidiError(ValueError):
    pass


@lru_cache(maxsize=1)
def library():
    override = os.environ.get('ORACLE_FRIBIDI_LIBRARY')
    candidates = [override] if override else [ctypes.util.find_library('fribidi')]
    if not override:
        if sys.platform == 'darwin':
            candidates += ['/opt/homebrew/lib/libfribidi.0.dylib', '/usr/local/lib/libfribidi.0.dylib']
        elif sys.platform == 'win32':
            candidates += ['libfribidi-0.dll']
        else:
            candidates += ['libfribidi.so.0']
    errors = []
    for candidate in filter(None, candidates):
        try:
            loaded = ctypes.CDLL(candidate)
        except OSError as error:
            errors.append(str(error))
            continue
        try:
            loaded.fribidi_get_bidi_type.argtypes = [ctypes.c_uint32]
            loaded.fribidi_get_bidi_type.restype = ctypes.c_uint32
            loaded.fribidi_log2vis.argtypes = [ctypes.POINTER(ctypes.c_uint32), ctypes.c_int,
                ctypes.POINTER(ctypes.c_uint32), ctypes.POINTER(ctypes.c_uint32),
                ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int8)]
            loaded.fribidi_log2vis.restype = ctypes.c_int8
            ctypes.c_char_p.in_dll(loaded, 'fribidi_version_info')
        except (AttributeError, ValueError) as error:
            raise BidiError(f'Invalid FriBidi library {candidate}: {error}') from error
        return loaded
    raise BidiError('FriBidi is required to verify RTL replacement text; install it or set '
                    'ORACLE_FRIBIDI_LIBRARY. ' + '; '.join(errors))


def provenance():
    loaded = library()
    return {'library': loaded._name,
            'version': ctypes.c_char_p.in_dll(loaded, 'fribidi_version_info').value.decode('utf-8')}


def visual_orders(text):
    """PDF does not retain paragraph direction; corroborate either UAX9 base direction."""
    if not text:
        return {''}
    loaded = library()
    source = (ctypes.c_uint32 * len(text))(*(ord(char) for char in text))
    visual = (ctypes.c_uint32 * len(text))()
    result = set()
    for strong in ('A', 'א'):
        direction = ctypes.c_uint32(loaded.fribidi_get_bidi_type(ord(strong)))
        if not loaded.fribidi_log2vis(source, len(text), ctypes.byref(direction), visual, None, None, None):
            raise BidiError('FriBidi could not resolve replacement-text ordering')
        result.add(''.join(chr(code) for code in visual))
    return result
