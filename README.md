My implementations for https://protohackers.com/  

```sh
cargo run -- 0 # run 0th (echo) problem
cargo run -- 3 # run 3rd problem
# etc ...
```

Port should be `3007`, but isn't guaranteed.  

Builds single binary to easily move things around between servers. I'll think about building seperate binaries when the binary size crosses `50MB` mark (currently it's around `8-9MB`)  
