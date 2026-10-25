# apidoc-idl

Generate `apidoc-rust` annotation source from `.proto` / `.thrift` IDL.

```bash
apidoc-idl proto  --in service.proto  --out generated.rs [--package <name>]
apidoc-idl thrift --in service.thrift --out generated.rs
```

The generated file is plain Rust containing `#[apidoc::grpc(...)]` / `#[apidoc::thrift(...)]`
functions plus `#[apidoc::param]` / `#[apidoc::returned]` fields; include or copy it into
your crate, then review it like any generated source before committing.

## Safety limits

- Input files are capped at **1 MiB** and recursive type nesting at **64** levels.
- Imports/includes are **never fetched or expanded**. Proto files may import only the
  built-in `google/protobuf/*` well-known types; other imports are rejected with a
  line-numbered error. Thrift `include` directives are emitted as comments.
- Generated identifiers are sanitized and string literals are escaped, so malformed
  IDL cannot inject arbitrary Rust code.
- The crate has no network access and reads no file other than the `--in` path.

## Not supported

Generic/unknown types are emitted with a `TODO` comment rather than guessed. Thrift
`const` values and `include` expansion are intentionally out of scope.
