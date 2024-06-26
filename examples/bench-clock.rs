use cover_circuit::{index_secret, Clock};
use plonky2::plonk::circuit_data::CircuitConfig;
use plonky2_maybe_rayon::rayon;
use rand::{seq::SliceRandom, thread_rng, Rng};
use tracing::info;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let mut config = CircuitConfig::standard_ecc_config();
    config.zero_knowledge = true;

    let num_thread = rayon::current_num_threads();
    info!(
        "Using {} compute threads on {:?} cores",
        num_thread,
        std::thread::available_parallelism(),
    );

    const S: usize = 1 << 10;
    let (clock, circuit) = Clock::<S>::genesis(
        [(); S].map({
            let mut i = 0;
            move |()| {
                let secret = index_secret(i);
                i += 1;
                cover_circuit::public_key(secret)
            }
        }),
        config,
    )?;
    clock.verify(&circuit)?;

    // let clock_bytes =
    //     std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("genesis_clock4.bin"))?;
    // let circuit_bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("circuit4.bin"))?;
    // let (clock, circuit) = Clock::<S>::from_bytes(clock_bytes, &circuit_bytes, config)?;

    let mut clocks = vec![clock];
    for _ in 0..10 {
        let clock1 = clocks.choose(&mut rand::thread_rng()).unwrap();
        let clock2 = clocks.choose(&mut rand::thread_rng()).unwrap();
        let index = thread_rng().gen_range(0..S);
        info!("updating {index} with {clock1:?} and {clock2:?}");
        // let start = Instant::now();
        let clock = clock1.update(index, index_secret(index), clock2, &circuit)?;
        info!("updated into {clock:?}");
        clock.verify(&circuit)?;
        clocks.push(clock)
    }
    Ok(())
}
