# gRPC and HTTP/1.1 co-hosting

An attempt at hosting HTTP/1.1, HTTP/2 and gRPC on the same port(s).

## gRPC testing

Use gRPC reflection to introspect the service: 

```shell
grpcurl --plaintext --use-reflection 127.0.0.1:50052 list
```

Send a test request:

```shell
grpcurl --plaintext --use-reflection -d '{ "message": "World" }' 127.0.0.1:50052 example.YourService/YourMethod
```
