# gRPC and HTTP/1.1 co-hosting

An attempt at hosting HTTP/1.1, HTTP/2 and gRPC on the same port(s).

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
