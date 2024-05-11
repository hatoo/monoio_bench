use std::sync::{atomic::AtomicUsize, atomic::Ordering::Relaxed, Arc};

use clap::Parser;
use http_body_util::{BodyExt, Empty};
use hyper::{body::Bytes, Request, Uri};
use monoio::{io::IntoPollIo, net::TcpStream, RuntimeBuilder};
use monoio_compat::hyper::MonoioIo;

#[derive(Parser, Debug)]
struct Opt {
    url: Uri,
    #[clap(default_value = "100", short)]
    n: usize,
    #[clap(default_value = "50", short)]
    c: usize,
}

fn main() {
    let opt = Opt::parse();

    let counter = Arc::new(AtomicUsize::new(0));

    let cpus = num_cpus::get();

    let now = std::time::Instant::now();

    let threads: Vec<_> = (0..cpus)
        .filter_map(|i| {
            let num_connection = opt.c / cpus + (if (opt.c % cpus) > i { 1 } else { 0 });

            if num_connection > 0 {
                let counter = counter.clone();
                let url = opt.url.clone();
                Some(std::thread::spawn(move || {
                    monoio::utils::bind_to_cpu_set(std::iter::once(i)).unwrap();
                    let mut rt = RuntimeBuilder::<monoio::IoUringDriver>::new()
                        // .with_entries(32768)
                        .build()
                        .unwrap();
                    rt.block_on(async move {
                        let futures: Vec<_> = (0..num_connection)
                            .map(|_| {
                                let counter = counter.clone();
                                let url = url.clone();
                                monoio::spawn(async move {
                                    loop {
                                        let Ok(stream) = TcpStream::connect((
                                            url.host().unwrap(),
                                            url.port_u16().unwrap_or(80),
                                        ))
                                        .await
                                        .and_then(|stream| stream.into_poll_io()) else {
                                            continue;
                                        };

                                        let io = MonoioIo::new(stream);
                                        let Ok((mut sender, conn)) =
                                            hyper::client::conn::http1::handshake(io).await
                                        else {
                                            continue;
                                        };
                                        monoio::spawn(conn);

                                        loop {
                                            if counter.fetch_add(1, Relaxed) < opt.n {
                                                let req = Request::get(&url)
                                                    .body(Empty::<Bytes>::new())
                                                    .unwrap();
                                                let Ok(mut res) = sender.send_request(req).await
                                                else {
                                                    break;
                                                };
                                                // dbg!(res.status());
                                                while let Some(_next) = res.frame().await {}
                                            } else {
                                                return;
                                            }
                                        }
                                    }
                                })
                            })
                            .collect();
                        for f in futures {
                            f.await;
                        }
                    });
                }))
            } else {
                None
            }
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }

    let elapsed = now.elapsed();

    println!(
        "{} requests in {:.2?}, {:.2} req/s",
        opt.n,
        elapsed,
        opt.n as f64 / elapsed.as_secs_f64()
    );
}
