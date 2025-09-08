```sh
wasm-pack build --target web

RUST_LOG=info cargo run --example desktop

cargo modules dependencies --no-externs --no-fns --no-uses | dot -Tsvg > ./graph.svg
```
