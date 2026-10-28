# Local TLS test credentials

These are disposable test credentials, generated locally with OpenSSL 3 using
P-256 named-curve keys. `localhost-key.der` is an unencrypted PKCS#8 test key;
`localhost.der` is a server certificate for DNS `localhost`, signed by `root.der`.
The test root is trusted only by the test client, never by the default loader.
Certificates are valid for ten years from 21 October 2026.

The tests run both server and client using rustls and RustCrypto. They verify
TLS 1.2 and TLS 1.3 and reject the same server with an untrusted issuer or an
IP-address hostname that is absent from the certificate's SAN. OpenSSL is not
a runtime or test execution dependency.
