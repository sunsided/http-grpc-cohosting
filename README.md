# gRPC and HTTP/1.1 co-hosting

An attempt at hosting HTTP/1.1, HTTP/2 and gRPC on the same port(s).

Multiple Hyper servers are spawned on different endpoints to showcase the use of binding to different IP addresses
and ports while reusing the same server components. A Hyper service is used to switch the incoming traffic based on the
`content-type` header and if `application/grpc` is detected, traffic is forwarded to the Tonic server; all other
cases forward to Axum. This allows for transparent use of HTTP/1.1 and HTTP/2 (prior knowledge), as well
as ALPN on the TLS-enabled ports.

This project uses:

- [Hyper] as the server for HTTP/1 and HTTP/2 support.
  - [tls-listener] with [tokio-rustls] is used to provide TLS with `h2` and `http/1.1` ALPN support.
- [Axum] as the HTTP server.
- [Tonic] as the gRPC server.

[Hyper]: https://github.com/hyperium/hyper
[Axum]: https://github.com/tokio-rs/axum
[Tonic]: https://github.com/hyperium/tonic
[tls-listener]: https://github.com/tmccombs/tls-listener
[tokio-rustls]: https://github.com/rustls/tokio-rustls

## Curl

### non-TLS

```shell
curl -v http://127.0.0.1:36849/
curl -v http://127.1.0.1:36849/
curl --http2-prior-knowledge --insecure -vv https://127.0.0.1:36849/
```

### TLS with ALPN

```shell
curl --insecure -v https://127.0.0.1:36850/
curl --http1.1 --insecure -vv https://127.0.0.1:36850/
curl --http2 --insecure -vv https://127.0.0.1:36850/
```

## nghttp (HTTP/2)

```shell
nghttp -v http://127.0.0.1:36849
nghttp -y -v https://127.0.0.1:36850
```

## gRPC testing

Use gRPC reflection to introspect the service: 

```shell
grpcurl --plaintext --use-reflection 127.0.0.1:36849 list
grpcurl --insecure --use-reflection 127.0.0.1:36850 list
```

Send a test request:

```shell
grpcurl --plaintext --use-reflection -d '{ "message": "World" }' 127.0.0.1:36849 example.YourService/YourMethod
grpcurl --insecure --use-reflection -d '{ "message": "World" }' 127.0.0.1:36850 example.YourService/YourMethod
```

## Recommended reads

The _Combining Axum, Hyper, Tonic, and Tower for hybrid web/gRPC apps_ series:

- [Part 1: Overview of Tower](https://www.fpcomplete.com/blog/axum-hyper-tonic-tower-part1/)
- [Part 2: Understanding Hyper, and first experiences with Axum](https://www.fpcomplete.com/blog/axum-hyper-tonic-tower-part2/)
- [Part 3: Demonstration of Tonic for a gRPC client/server](https://www.fpcomplete.com/blog/axum-hyper-tonic-tower-part3/)
- [Part 4: How to combine Axum and Tonic services into a single service](https://www.fpcomplete.com/blog/axum-hyper-tonic-tower-part4/)
