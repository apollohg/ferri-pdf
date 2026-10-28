"""Bounded child execution, artifact classification and reproducible run records."""
import bidi_text
from datetime import datetime, timezone
import hashlib
from importlib.metadata import PackageNotFoundError, version
import json
import math
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import tempfile
import threading
import time
import uuid

DEFAULT_COMMAND_TIMEOUT_S = 120
REPORT_SCHEMA_VERSION = 1
EXIT_FAILURE = 1
EXIT_INTERRUPTED = 130
HOST_PROBE_TIMEOUT_S = 5
KILL_GRACE_S = 1
MILLISECONDS_PER_SECOND = 1000
FILE_HASH_CHUNK_BYTES = 1024 * 1024
PDF_HEADER = b'%PDF-'
PDF_END = b'%%EOF'
COMMAND_COMPLETED = 'completed'
COMMAND_TIMEOUT = 'timeout'
COMMAND_INTERRUPTED = 'interrupted'
INFRASTRUCTURE_ERROR = 'infrastructure_error'
RENDERED = 'rendered'
UNSUPPORTED = 'unsupported'
RENDER_ERROR = 'render_error'
NOT_RUN = 'not_run'
UNSUPPORTED_RUST = re.compile(r'^(?:CSS: unsupported CSS:|document formatting: unsupported (?:formatting element:|display |generated display:)|layout: unsupported (?:table formatting at block |flex formatting:))')


class CommandError(RuntimeError):
    def __init__(self, record):
        self.record = record
        detail = record.get('error') or read_log(record, 'stderr')
        super().__init__(f"command {record['status']} (exit {record['exit_status']}): {record['argv']}\n{detail}")


def read_log(record: dict, stream: str) -> str:
    path = record.get(f'{stream}_path')
    return Path(path).read_text(encoding='utf-8', errors='replace') if path else ''


def _signal_group(process, *, force):
    try:
        if os.name == 'posix':
            os.killpg(process.pid, signal.SIGKILL if force else signal.SIGTERM)
        elif force:
            process.kill()
        else:
            process.terminate()
    except ProcessLookupError:
        pass


def _stop_group(process):
    _signal_group(process,force=False)
    try:
        process.wait(timeout=KILL_GRACE_S)
    except subprocess.TimeoutExpired:
        pass
    _signal_group(process,force=True)
    process.wait()


def _deadline(process, timed_out):
    if process.returncode is None:
        timed_out.set()
        _signal_group(process,force=True)


def rss_bytes(value, system):
    return None if value is None else int(value)*(1 if system=='Darwin' else 1024)


def run_command(argv: list[str], output_dir: Path, timeout_s: float, *, collect_rss=False) -> dict:
    if isinstance(timeout_s, bool) or not isinstance(timeout_s, (int,float)) or not math.isfinite(timeout_s) or timeout_s <= 0:
        raise ValueError('command timeout must be positive and finite')
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    identity = uuid.uuid4().hex
    stdout_path, stderr_path = (output_dir/f'{identity}.{stream}.log' for stream in ('stdout','stderr'))
    record = {
        'argv': list(map(str,argv)), 'status': COMMAND_COMPLETED, 'exit_status': None,
        'stdout_path': str(stdout_path), 'stderr_path': str(stderr_path),
        'started_at': datetime.now(timezone.utc).isoformat(), 'started_ns': time.time_ns(),
        'elapsed_ms': None, 'pid': None,
    }
    with stdout_path.open('wb') as stdout, stderr_path.open('wb') as stderr:
        start = time.perf_counter()
        end = None
        try:
            process = subprocess.Popen(record['argv'], stdout=stdout, stderr=stderr, start_new_session=os.name=='posix')
        except OSError as error:
            record.update(status=INFRASTRUCTURE_ERROR, error=str(error))
            end = time.perf_counter()
        else:
            record['pid'] = process.pid
            timed_out = threading.Event()
            timer = threading.Timer(max(0,timeout_s-(time.perf_counter()-start)),_deadline,args=(process,timed_out))
            timer.start()
            try:
                if collect_rss and hasattr(os,'wait4'):
                    _, status, usage = os.wait4(process.pid,0)
                    process.returncode = os.waitstatus_to_exitcode(status)
                    record['peak_rss_bytes'] = rss_bytes(usage.ru_maxrss,platform.system())
                    record['exit_status'] = process.returncode
                else:
                    record['exit_status'] = process.wait()
                    if collect_rss:
                        record['peak_rss_bytes'] = None
                        record['rss_unavailable_reason'] = 'Per-child wait4 is unavailable'
                end = time.perf_counter()
                if timed_out.is_set():
                    record['status'] = COMMAND_TIMEOUT
            except KeyboardInterrupt:
                record['status'] = COMMAND_INTERRUPTED
                end = time.perf_counter()
            finally:
                timer.cancel()
                timer.join()
                _stop_group(process)
                record['exit_status'] = process.returncode
    record['elapsed_ms'] = (end-start)*MILLISECONDS_PER_SECOND
    return record


def checked_command(argv: list[str], output_dir: Path, timeout_s: float) -> tuple[str, str]:
    record = run_command(argv, output_dir, timeout_s)
    if record['status'] == COMMAND_INTERRUPTED:
        raise KeyboardInterrupt
    if record['status'] != COMMAND_COMPLETED or record['exit_status']:
        raise CommandError(record)
    return read_log(record,'stdout'), read_log(record,'stderr')


def classify_render(record: dict, pdf: Path) -> str:
    if record['status'] in (COMMAND_TIMEOUT, INFRASTRUCTURE_ERROR):
        return record['status']
    if record['status'] == COMMAND_INTERRUPTED:
        return RENDER_ERROR
    if record['exit_status']:
        lines = read_log(record,'stderr').strip().splitlines()
        if record['exit_status'] > 0 and lines and UNSUPPORTED_RUST.match(lines[0]):
            return UNSUPPORTED
        return RENDER_ERROR
    try:
        if Path(pdf).stat().st_mtime_ns < record['started_ns']:
            return RENDER_ERROR
        with Path(pdf).open('rb') as stream:
            header = stream.read(len(PDF_HEADER))
            stream.seek(0,os.SEEK_END)
            size = stream.tell()
            stream.seek(max(0,size-FILE_HASH_CHUNK_BYTES))
            ending = stream.read().rstrip()
    except OSError:
        return RENDER_ERROR
    if header != PDF_HEADER or not ending.endswith(PDF_END):
        return RENDER_ERROR
    return RENDERED


def render_diagnostics(log,dispositions=()):
    unresolved = []
    warnings = []
    for line in log.splitlines():
        if 'warning:' in line.lower() or 'error:' in line.lower():
            warnings.append(line)
            accepted = any(item.get('status')=='accepted' and item.get('reason') and re.fullmatch(item['pattern'],line) for item in dispositions)
            if not accepted:unresolved.append(line)
    return {'warnings':warnings,'warnings_unresolved':bool(unresolved),'unresolved_warnings':unresolved}


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda:stream.read(FILE_HASH_CHUNK_BYTES),b''):
            hasher.update(chunk)
    return hasher.hexdigest()


def fingerprint_files(paths):
    files = {}
    for path in paths:
        path = Path(path)
        entries = sorted(path.rglob('*')) if path.is_dir() else [path]
        for entry in entries:
            if path.is_dir() and (not entry.is_file() or any(part in {'.git','.claude','.superpowers','__pycache__','target','output'} for part in entry.relative_to(path).parts)):
                continue
            try:files[str(entry.absolute())] = digest(entry)
            except OSError:files[str(entry.absolute())] = None
    return files


def capture_provenance(root: Path, paths: list[Path]) -> dict:
    root = Path(root).resolve()
    def git(*args):
        completed = subprocess.run(['git','-C',str(root),*args],capture_output=True,timeout=DEFAULT_COMMAND_TIMEOUT_S)
        return completed.stdout if not completed.returncode else b''
    files = fingerprint_files(paths)
    diff = git('diff','--binary','HEAD')
    if not git('rev-parse','--verify','HEAD'):
        diff = git('diff','--binary','--cached') + git('diff','--binary')
    try:
        memory = os.sysconf('SC_PHYS_PAGES')*os.sysconf('SC_PAGE_SIZE')
    except (ValueError, OSError, AttributeError):
        memory = None
    try:
        available_memory = os.sysconf('SC_AVPHYS_PAGES')*os.sysconf('SC_PAGE_SIZE')
    except (ValueError, OSError, AttributeError):
        available_memory = None
    cpu = platform.processor() or platform.machine()
    if platform.system() == 'Darwin':
        probe = subprocess.run(['sysctl','-n','machdep.cpu.brand_string'],capture_output=True,text=True,timeout=HOST_PROBE_TIMEOUT_S)
        if not probe.returncode:
            cpu = probe.stdout.strip()
    dependencies = {}
    for package in ('Pillow','fonttools'):
        try:
            dependencies[package] = version(package)
        except PackageNotFoundError:
            dependencies[package] = None
    try:
        native_bidi = bidi_text.provenance()
    except bidi_text.BidiError as error:
        native_bidi = {'error': str(error)}
    return {
        'native_dependencies': {'fribidi': native_bidi},
        'python_dependencies': dependencies,
        'source_commit': git('rev-parse','HEAD').decode().strip() or None,
        'dirty_diff_sha256': hashlib.sha256(diff).hexdigest(),
        'files': files, 'python': platform.python_version(),
        'machine': {**platform.uname()._asdict(), 'cpu': cpu, 'memory_bytes': memory, 'available_memory_bytes': available_memory},
        'recorded_at': datetime.now(timezone.utc).isoformat(),
    }


def write_report(path: Path, report: dict) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8', dir=path.parent, suffix='.tmp', delete=False) as stream:
            temporary = Path(stream.name)
            json.dump(report,stream,indent=2,ensure_ascii=False)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        temporary.replace(path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
