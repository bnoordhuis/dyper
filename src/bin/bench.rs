use std::collections::HashMap;
use std::io;
use std::time;

// TODO(bnoordhuis) Make configurable.
const NUM_CLIENTS: usize = 40;
const NUM_REQUESTS: usize = 400;

const HTTP_ADDR: &str = "http://127.0.0.1:4000/";

fn main() {
  tokio::runtime::Builder::new_multi_thread()
    .enable_io()
    .enable_time()
    .worker_threads(2) // Mimics wrk setup in deno benchmarks.
    .max_blocking_threads(8)
    .build()
    .unwrap()
    .block_on(async_main())
    .unwrap()
}

async fn async_main() -> io::Result<()> {
  let mut clients = vec![];

  for _ in 0..NUM_CLIENTS {
    clients.push(tokio::spawn(pummel()));
  }

  let mut samples = vec![];

  for client in clients {
    samples.append(&mut client.await.unwrap());
  }

  let _error_count = samples.iter().filter(|sample| sample.is_err()).count();

  for err in samples.iter().filter(|sample| sample.is_err()) {
    eprintln!("{:?}", err);
  }

  let samples: Vec<_> = samples.into_iter().filter_map(Result::ok).collect();

  let mut status_codes = HashMap::new();

  for sample in &samples {
    status_codes
      .entry(sample.status)
      .and_modify(|e| *e += 1)
      .or_insert(1);
  }

  let mut status_codes: Vec<_> = status_codes.drain().collect();

  status_codes.sort_unstable();

  for (status, count) in status_codes {
    let mut samples: Vec<_> = samples
      .iter()
      .filter(|e| e.status == status)
      .map(|e| e.t1.duration_since(e.t0).unwrap().as_micros() as u64)
      .collect();

    samples.sort_unstable();

    let min = samples.first().unwrap();
    let max = samples.last().unwrap();

    let avg: u64 = samples.iter().sum();
    let avg = avg / samples.len() as u64;

    let weighed = |x: &u64| {
      let x = *x as i64 - avg as i64;
      x * x
    };

    let stddev: i64 = samples.iter().map(weighed).sum();
    let stddev = stddev / samples.len() as i64;
    let stddev = (stddev as f64).sqrt();

    println!(
      "  status={} min={} avg={} max={} stddev={:.2} count={} (times in us)",
      status, min, avg, max, stddev, count
    );
  }

  Ok(())
}

async fn pummel() -> Vec<reqwest::Result<Sample>> {
  let mut client = reqwest::Client::builder().build().unwrap();

  let mut samples = vec![];

  for _ in 0..NUM_REQUESTS {
    samples.push(pummel1(&mut client).await);
  }

  samples
}

async fn pummel1(client: &mut reqwest::Client) -> reqwest::Result<Sample> {
  let t0 = time::SystemTime::now();
  let res = client.get(HTTP_ADDR).send().await?;
  let status = res.status().as_u16();
  let _body = res.bytes().await?;
  let t1 = time::SystemTime::now();
  Ok(Sample { t0, t1, status })
}

#[derive(Debug)]
struct Sample {
  t0: time::SystemTime,
  t1: time::SystemTime,
  status: u16,
}
