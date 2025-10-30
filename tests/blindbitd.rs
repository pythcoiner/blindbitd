use blindbitd::BlindbitD;

fn new_blindbitd_instance() -> BlindbitD {
    let blindbitd = BlindbitD::new().unwrap();
    println!("BlindbitD running at {}:{}", blindbitd.addr, blindbitd.port);
    blindbitd
}

#[test]
fn simple_blindbitd() {
    let mut bbd = new_blindbitd_instance();
    let mut node = bbd.bitcoin().unwrap();
    let bitcoind = &mut node.client;
    let address = bitcoind.new_address().unwrap();
    // Generate 100 blocks
    bitcoind.generate_to_address(100, &address).unwrap();
}
