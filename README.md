# blindbitd

Utility to run an [blindbit-oracle](https://github.com/setavenger/blindbit-oracle)
 instance into rust integration tests.

It will spawn a [corepc-node](https://github.com/rust-bitcoin/corepc)
under the hood for convenience.

This repo is (largely) inspired from [bitcoind](https://github.com/rust-bitcoin/bitcoind) & 
[electrsd](https://github.com/RCasatta/electrsd) projects.

# Binaries

This lib is shipped with a [blindbit-oracle](https://github.com/setavenger/blindbit-oracle)
linux binary that i've compiled myself from commit bcd562f for convenience but you should use binaries you build by yourself.

# Usage

## Running with the supplied linux binary

```rust
use blindbitd::BlindbitD;

let mut bbd = new_blindbitd_instance();
let bitcoind = bbd.bitcoin();

// Generate 100 blocks
let address = bitcoind.new_address().unwrap();
bitcoind.generate_to_address(100, &address).unwrap();

let url = bbd.url();
```

## Running with your binary

```rust
use blindbitd::{BlindbitD, Conf};

let mut conf = Conf::default();
conf.binary = Some("path/to/your/binary".into());

let bbd = BlindbitD::with_conf(&conf).unwrap();

let mut bbd = new_blindbitd_instance();
let bitcoind = bbd.bitcoin();

// Generate 100 blocks
let address = bitcoind.new_address().unwrap();
bitcoind.generate_to_address(100, &address).unwrap();

let url = bbd.url();
```

