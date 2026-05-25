# masterbus-ffi

A single-threaded **C ABI** over the [`masterbus`](https://crates.io/crates/masterbus)
crate: open a SocketCAN connection, enumerate devices, read fields, and write
booleans/floats from C. The header `include/masterbus.h` is generated with
cbindgen at build time.

Building the crate produces a `libmasterbus_ffi` C library (`cdylib` +
`staticlib`). Example C programs and a cross-compiling `Makefile` live under
[`c/`](c).

```c
MbBus *bus = mb_open_socketcan("can0", NULL);
uint32_t *ids; int32_t n = mb_devices(bus, &ids);
for (int32_t i = 0; i < n; i++) {
    char *name = mb_device_name(bus, ids[i]);
    printf("%u: %s\n", ids[i], name ? name : "?");
    mb_free_str(name);
}
mb_free_ids(ids, n);
mb_close(bus);
```

**Contract:** a context must only be used from one thread. See the
[repository](https://github.com/keesverruijt/masterbus) for details.

## License

Apache-2.0.
