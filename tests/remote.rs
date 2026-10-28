use ferri_pdf::{
    FileResolver, LoaderOptions, RenderOptions, ResourceBase, ResourceError, ResourceResolver, Url,
    UrlResolver, render_with_resolver,
};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const OK: &str = "200 OK";
const FOUND: &str = "302 Found";
const NOT_FOUND: &str = "404 Not Found";
const POLL_INTERVAL: Duration = Duration::from_millis(2);
struct Reply {
    status: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    delay: Duration,
}
impl Reply {
    fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: OK,
            headers: Vec::new(),
            body: body.into(),
            delay: Duration::ZERO,
        }
    }
    fn redirect(location: &str) -> Self {
        Self {
            status: FOUND,
            headers: vec![("Location", location.into())],
            ..Self::ok([])
        }
    }
}
struct Server {
    base: Url,
    calls: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    fn new(routes: impl IntoIterator<Item = (&'static str, Reply)>) -> Self {
        let routes: HashMap<_, _> = routes.into_iter().collect();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = Url::parse(&format!(
            "http://{}/document.html",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let recorded = calls.clone();
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => serve(stream, &routes, &recorded),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(POLL_INTERVAL)
                    }
                    Err(error) => panic!("HTTP test server accept: {error}"),
                }
            }
        });
        Self {
            base,
            calls,
            stop,
            worker: Some(worker),
        }
    }
    fn requests(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}
fn serve(mut stream: TcpStream, routes: &HashMap<&str, Reply>, calls: &Mutex<Vec<String>>) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let path = first
        .split_whitespace()
        .nth(1)
        .expect("request target")
        .to_owned();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header == "\r\n" || header.is_empty() {
            break;
        }
    }
    calls.lock().unwrap().push(path.clone());
    let missing = Reply {
        status: NOT_FOUND,
        ..Reply::ok([])
    };
    let reply = routes.get(path.as_str()).unwrap_or(&missing);
    thread::sleep(reply.delay);
    let mut response = format!(
        "HTTP/1.1 {}\r\nConnection: close\r\nContent-Length: {}\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    let result = stream
        .write_all(response.as_bytes())
        .and_then(|_| stream.write_all(&reply.body));
    if let Err(error) = result {
        assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ),
            "HTTP test write: {error}"
        );
    }
}

#[test]
fn redirects_preserve_the_final_base_and_check_policy_before_network_access() {
    let server = Server::new([
        ("/start", Reply::redirect("/assets/outer.svg")),
        ("/assets/outer.svg", Reply::ok(b"svg")),
        ("/blocked", Reply::ok(b"forbidden")),
        ("/escape", Reply::redirect("/blocked")),
    ]);
    let checked = Arc::new(Mutex::new(Vec::new()));
    let capture = checked.clone();
    let resolver = UrlResolver::default().with_url_filter(move |url| {
        capture.lock().unwrap().push(url.path().to_owned());
        url.path() != "/blocked"
    });
    let base = server.base.clone().into();
    let resource = resolver.resolve("start", &base).unwrap();
    assert_eq!(resource.bytes, b"svg");
    assert_eq!(
        resource.base.resolve("child.png").unwrap(),
        server.base.join("/assets/child.png").unwrap()
    );
    assert!(matches!(
        resolver.resolve("escape", &base),
        Err(ResourceError::Denied(_))
    ));
    assert_eq!(
        server.requests(),
        ["/start", "/assets/outer.svg", "/escape"],
        "blocked redirect destination must receive no request"
    );
    assert_eq!(
        *checked.lock().unwrap(),
        ["/start", "/assets/outer.svg", "/escape", "/blocked"]
    );
}

#[test]
fn remote_stylesheets_fonts_and_nested_svg_use_the_shared_loader_and_final_urls() {
    let server=Server::new([
        ("/style.css",Reply::redirect("/theme/main.css")),
        ("/theme/main.css",Reply::ok(b"@font-face{font-family:Remote;src:url(font.ttf)}body{font-family:Remote}img{display:block}")),
        ("/theme/font.ttf",Reply::ok(include_bytes!("oracle/assets/DejaVuSans.ttf").as_slice())),
        ("/picture.svg",Reply::redirect("/media/picture.svg")),
        ("/media/picture.svg",Reply::ok(br#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40" fill="red"/><image href="pixels.heic" width="80" height="40"/></svg>"#)),
        ("/media/pixels.heic",Reply::ok(include_bytes!("oracle/assets/image-pattern.heic").as_slice())),
    ]);
    let checked = Arc::new(Mutex::new(Vec::new()));
    let capture = checked.clone();
    let resolver = UrlResolver::default().with_url_filter(move |url| {
        capture.lock().unwrap().push(url.path().to_owned());
        true
    });
    let options = RenderOptions {
        base: server.base.clone().into(),
        include_layout: true,
        ..Default::default()
    };
    let output=render_with_resolver("<link rel=stylesheet href='style.css'><p>Remote resources</p><img src='picture.svg'><img src='picture.svg'>",&options,&resolver).unwrap();
    assert_eq!(output.page_count, 1);
    let trace = output.layout.unwrap();
    assert!(
        trace.fonts.iter().any(|font| font.family == "Remote"),
        "remote font must be loaded rather than replaced by fallback"
    );
    let requests = server.requests();
    for path in [
        "/style.css",
        "/theme/main.css",
        "/theme/font.ttf",
        "/picture.svg",
        "/media/picture.svg",
        "/media/pixels.heic",
    ] {
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.as_str() == path)
                .count(),
            1,
            "{path} should be fetched once: {requests:?}"
        );
    }
    assert_eq!(
        *checked.lock().unwrap(),
        requests,
        "all actual fetches must pass the filter"
    );
}

#[test]
fn denied_nested_svg_resource_is_reported_without_fetching_it() {
    let server=Server::new([("/outer.svg",Reply::ok(br#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><image href="blocked.png" width="80" height="40"/></svg>"#))]);
    let resolver = UrlResolver::default().with_url_filter(|url| url.path() != "/blocked.png");
    let options = RenderOptions {
        base: server.base.clone().into(),
        ..Default::default()
    };
    let result = render_with_resolver(
        "<style>img{display:block}</style><img src='outer.svg'>",
        &options,
        &resolver,
    );
    let output = result.unwrap();
    assert!(
        matches!(output.diagnostics.as_slice(), [ferri_pdf::RenderDiagnostic::UnavailableImage(failure)]
        if matches!(&failure.error, ferri_pdf::ImageError::Resource(ResourceError::Denied(_))))
    );
    assert_eq!(server.requests(), ["/outer.svg"]);
}

#[test]
fn remote_limits_cover_body_size_redirect_loops_status_and_timeout() {
    let mut slow = Reply::ok(b"late");
    slow.delay = Duration::from_millis(100);
    let server = Server::new([
        ("/exact", Reply::ok(b"1234")),
        ("/large", Reply::ok(b"12345")),
        ("/loop", Reply::redirect("loop")),
        ("/slow", slow),
    ]);
    let base = server.base.clone().into();
    let resolver = UrlResolver::new(LoaderOptions {
        files: FileResolver {
            max_resource_bytes: 4,
            ..Default::default()
        },
        max_redirects: 2,
        ..Default::default()
    });
    assert_eq!(resolver.resolve("exact", &base).unwrap().bytes, b"1234");
    assert!(matches!(
        resolver.resolve("large", &base),
        Err(ResourceError::RemoteTooLarge { limit: 4, .. })
    ));
    assert!(matches!(
        resolver.resolve("loop", &base),
        Err(ResourceError::RedirectLimit { limit: 2, .. })
    ));
    assert!(matches!(
        resolver.resolve("missing", &base),
        Err(ResourceError::HttpStatus { status: 404, .. })
    ));
    let resolver = UrlResolver::new(LoaderOptions {
        timeout: Duration::from_millis(20),
        ..Default::default()
    });
    assert!(matches!(
        resolver.resolve("slow", &base),
        Err(ResourceError::Timeout(_))
    ));
}

#[test]
fn file_urls_decode_paths_and_keep_canonical_root_checks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let url = Url::from_file_path(root.join("tests/oracle/assets/image-pattern.png")).unwrap();
    let resolver = UrlResolver::new(LoaderOptions {
        files: FileResolver {
            root: Some(root.join("tests/oracle/assets")),
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(
        resolver
            .resolve(url.as_str(), &ResourceBase::default())
            .unwrap()
            .bytes,
        include_bytes!("oracle/assets/image-pattern.png")
    );
    assert!(matches!(
        resolver.resolve("../../Cargo.toml", &root.join("tests/oracle").into()),
        Err(ResourceError::OutsideRoot(_))
    ));
    let resolver = UrlResolver::default().with_url_filter(|url| url.scheme() != "file");
    assert!(matches!(
        resolver.resolve(url.as_str(), &ResourceBase::default()),
        Err(ResourceError::Denied(_))
    ));
}

#[test]
fn compressed_response_limit_applies_after_gzip_expansion() {
    let mut reply = Reply::ok(include_bytes!("oracle/assets/resource-expanded.gz").as_slice());
    reply.headers.push(("Content-Encoding", "gzip".into()));
    let server = Server::new([("/compressed", reply)]);
    let resolver = UrlResolver::new(LoaderOptions {
        files: FileResolver {
            max_resource_bytes: 64,
            ..Default::default()
        },
        ..Default::default()
    });
    assert!(
        matches!(
            resolver.resolve("compressed", &server.base.clone().into()),
            Err(ResourceError::RemoteTooLarge { limit: 64, .. })
        ),
        "small compressed transfer must not evade the decoded body limit"
    );
}

#[test]
fn remote_font_fallback_skips_missing_sources_but_never_policy_denials() {
    let server = Server::new([(
        "/valid.ttf",
        Reply::ok(include_bytes!("oracle/assets/DejaVuSans.ttf").as_slice()),
    )]);
    let options = RenderOptions {
        base: server.base.clone().into(),
        include_layout: true,
        ..Default::default()
    };
    let html = "<style>@font-face{font-family:Remote;src:url(missing.ttf),url(valid.ttf)}body{font-family:Remote}</style><p>Fallback source</p>";
    let output = render_with_resolver(html, &options, &UrlResolver::default()).unwrap();
    assert!(
        output
            .layout
            .unwrap()
            .fonts
            .iter()
            .any(|font| font.family == "Remote")
    );
    assert_eq!(server.requests(), ["/missing.ttf", "/valid.ttf"]);
    let resolver = UrlResolver::default().with_url_filter(|url| url.path() != "/missing.ttf");
    let error = render_with_resolver(html, &options, &resolver)
        .err()
        .expect("policy denial must not fall back");
    assert!(
        matches!(
            error,
            ferri_pdf::Error::Css(ferri_pdf::StyleError::Resource(ResourceError::Denied(_)))
        ),
        "policy error must propagate: {error:?}"
    );
    assert_eq!(
        server.requests(),
        ["/missing.ttf", "/valid.ttf"],
        "denial must prevent both the denied request and fallback request"
    );
}

#[test]
fn first_html_base_href_controls_document_resources() {
    let server = Server::new([(
        "/assets/picture.heic",
        Reply::ok(include_bytes!("oracle/assets/image-pattern.heic").as_slice()),
    )]);
    let options = RenderOptions {
        base: server.base.clone().into(),
        ..Default::default()
    };
    let output=render_with_resolver("<base target='_blank'><base href='assets/'><base href='ignored/'><style>img{display:block}</style><img src='picture.heic'>",&options,&UrlResolver::default()).unwrap();
    assert_eq!(output.page_count, 1);
    assert_eq!(server.requests(), ["/assets/picture.heic"]);
}

#[test]
fn http_metadata_uses_one_policy_checked_fetch() {
    const START: &str = "/metadata-start";
    const CSS: &str = "/theme/metadata.css";
    const BYTES: &[u8] = b"p::before{content:'caf\xe9'}";
    let mut reply = Reply::ok(BYTES);
    reply.headers.push((
        "Content-Type",
        "Text/CSS; note=\"a;charset=wrong\"; ChArSeT=\"windows-1252\"; charset=utf-8".into(),
    ));
    let server = Server::new([(START, Reply::redirect(CSS)), (CSS, reply)]);
    let checked = Arc::new(Mutex::new(Vec::new()));
    let capture = checked.clone();
    let resolver = UrlResolver::default().with_url_filter(move |url| {
        capture.lock().unwrap().push(url.path().to_owned());
        true
    });
    let resolved = resolver
        .resolve_with_metadata(START, &server.base.clone().into())
        .unwrap();
    assert_eq!(
        resolved.resource.bytes, BYTES,
        "transport decoding must preserve source bytes"
    );
    assert_eq!(
        resolved.resource.base.resolve("font.ttf").unwrap(),
        server.base.join("/theme/font.ttf").unwrap()
    );
    assert_eq!(resolved.metadata.media_type.as_deref(), Some("text/css"));
    assert_eq!(resolved.metadata.charset.as_deref(), Some("windows-1252"));
    assert_eq!(
        server.requests(),
        [START, CSS],
        "metadata must not fetch the body twice"
    );
    assert_eq!(*checked.lock().unwrap(), [START, CSS]);
}

#[test]
fn http_metadata_parses_mime_parameters_without_changing_body_bytes() {
    const CSS: &str = "/metadata.css";
    for (header, media_type, charset) in [
        (None, None, None),
        (Some("text/css"), Some("text/css"), None),
        (
            Some(" TeXT/CSS ; charset=windows-1252 "),
            Some("text/css"),
            Some("windows-1252"),
        ),
        (
            Some("text/css; charset=\"windows\\-1252\""),
            Some("text/css"),
            Some("windows-1252"),
        ),
        (
            Some("text/css; charset=; charset=utf-8"),
            Some("text/css"),
            Some("utf-8"),
        ),
        (
            Some("text/css; charset=\"\"; charset=utf-8"),
            Some("text/css"),
            Some(""),
        ),
        (
            Some("text/css; charset=\"shift_jis\"ignored; charset=utf-8"),
            Some("text/css"),
            Some("shift_jis"),
        ),
        (
            Some("text/css; charset=\"utf-8"),
            Some("text/css"),
            Some("utf-8"),
        ),
        (
            Some("text/css; charset =wrong; charset=utf-8"),
            Some("text/css"),
            Some("utf-8"),
        ),
        (
            Some("text/css; unknown; charset=utf-8"),
            Some("text/css"),
            Some("utf-8"),
        ),
        (Some("not-a-media-type; charset=utf-8"), None, None),
        (Some("text/; charset=utf-8"), None, None),
        (Some("text/c ss; charset=utf-8"), None, None),
    ] {
        let mut reply = Reply::ok(b"source bytes");
        if let Some(value) = header {
            reply.headers.push(("Content-Type", value.into()));
        }
        let server = Server::new([(CSS, reply)]);
        let resolved = UrlResolver::default()
            .resolve_with_metadata(CSS, &server.base.clone().into())
            .unwrap();
        assert_eq!(
            resolved.metadata.media_type.as_deref(),
            media_type,
            "media type from {header:?}"
        );
        assert_eq!(
            resolved.metadata.charset.as_deref(),
            charset,
            "charset from {header:?}"
        );
        assert_eq!(resolved.resource.bytes, b"source bytes");
        assert_eq!(server.requests(), [CSS]);
    }
}

#[test]
fn http_metadata_enforces_redirect_and_expanded_body_policy() {
    const ESCAPE: &str = "/metadata-escape";
    const BLOCKED: &str = "/metadata-blocked";
    const COMPRESSED: &str = "/metadata-compressed";
    const BYTE_LIMIT: usize = 64;
    let mut compressed = Reply::ok(include_bytes!("oracle/assets/resource-expanded.gz").as_slice());
    compressed.headers.push(("Content-Encoding", "gzip".into()));
    compressed
        .headers
        .push(("Content-Type", "text/css; charset=utf-8".into()));
    let server = Server::new([
        (ESCAPE, Reply::redirect(BLOCKED)),
        (BLOCKED, Reply::ok(b"forbidden")),
        (COMPRESSED, compressed),
    ]);
    let resolver = UrlResolver::new(LoaderOptions {
        files: FileResolver {
            max_resource_bytes: BYTE_LIMIT,
            ..Default::default()
        },
        ..Default::default()
    })
    .with_url_filter(|url| url.path() != BLOCKED);
    let base = server.base.clone().into();
    assert!(matches!(
        resolver.resolve_with_metadata(ESCAPE, &base),
        Err(ResourceError::Denied(_))
    ));
    assert!(matches!(
        resolver.resolve_with_metadata(COMPRESSED, &base),
        Err(ResourceError::RemoteTooLarge {
            limit: BYTE_LIMIT,
            ..
        })
    ));
    assert_eq!(
        server.requests(),
        [ESCAPE, COMPRESSED],
        "metadata must never bypass URL or expanded-body limits"
    );
}
