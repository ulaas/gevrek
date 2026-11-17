//! The few lines that time a case, for the benches of this tree (the
//! engine's, the server's): no crate is used for it. A case is run until
//! it fills a sample, thirty samples are taken, and the median and the
//! quickest are told.

use std::time::{Duration, Instant};

const SAMPLES: usize = 30;
/// How long one sample runs at least: short runs are repeated until
/// they fill it, so that the clock's grain does not show.
const SAMPLE: Duration = Duration::from_millis(5);

pub struct Bench {
    only: Option<String>,
    rows: Vec<(String, u128, u128)>,
}

impl Bench {
    /// (`cargo bench` passes `--bench`; a name is what does not begin
    /// with a dash.)
    pub fn from_args() -> Self {
        let only = std::env::args().skip(1).find(|word| !word.starts_with('-'));
        println!("{:<44} {:>14} {:>14}", "one run of", "median", "quickest");
        Self { only, rows: Vec::new() }
    }

    /// With `GEVREK_BENCH_OUT=<file>`: the medians are added to that
    /// file, a name and its nanoseconds on each line.
    pub fn written(&self) {
        use std::io::Write;
        let Some(out) = std::env::var_os("GEVREK_BENCH_OUT") else {
            return;
        };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(out)
            .expect("GEVREK_BENCH_OUT: a file that can be written");
        for (name, median, _) in &self.rows {
            writeln!(file, "{name}	{median}").expect("written");
        }
    }

    pub fn case(&mut self, name: &str, mut run: impl FnMut()) {
        if self.only.as_ref().is_some_and(|only| !name.contains(only.as_str())) {
            return;
        }
        // How many runs fill a sample (which also warms it up).
        let mut runs: u32 = 1;
        loop {
            let began = Instant::now();
            for _ in 0..runs {
                run();
            }
            if began.elapsed() >= SAMPLE || runs >= 1 << 24 {
                break;
            }
            runs *= 2;
        }
        let mut samples: Vec<u128> = (0..SAMPLES)
            .map(|_| {
                let began = Instant::now();
                for _ in 0..runs {
                    run();
                }
                began.elapsed().as_nanos() / u128::from(runs)
            })
            .collect();
        samples.sort_unstable();
        let (median, quickest) = (samples[SAMPLES / 2], samples[0]);
        println!("{name:<44} {:>14} {:>14}", time(median), time(quickest));
        self.rows.push((name.to_string(), median, quickest));
    }
}

/// Nanoseconds as they are read: `812 ns`, `14.2 µs`, `3.41 ms`.
fn time(nanos: u128) -> String {
    match nanos {
        0..=999 => format!("{nanos} ns"),
        1_000..=999_999 => format!("{:.1} µs", nanos as f64 / 1e3),
        _ => format!("{:.2} ms", nanos as f64 / 1e6),
    }
}

