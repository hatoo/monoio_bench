use std::sync::{atomic::AtomicUsize, atomic::Ordering::Relaxed, Arc};

use clap::Parser;
use http_body_util::{BodyExt, Empty};
use hyper::{body::Bytes, Request, Uri};
use monoio::{io::IntoPollIo, net::TcpStream};
use monoio_compat::hyper::MonoioIo;

#[derive(Parser, Debug)]
struct Opt {
    url: Uri,
    #[clap(default_value = "100", short)]
    n: usize,
    #[clap(default_value = "50", short)]
    c: usize,
}

#[monoio::main]
async fn main() {
    let opt = Opt::parse();

    let counter = Arc::new(AtomicUsize::new(0));

    let now = std::time::Instant::now();
    let futures: Vec<_> = (0..opt.c)
        .map(|_| {
            let counter = counter.clone();
            let url = opt.url.clone();

            monoio::spawn(async move {
                loop {
                    let Ok(stream) =
                        TcpStream::connect((url.host().unwrap(), url.port_u16().unwrap_or(80)))
                            .await
                            .and_then(|stream| stream.into_poll_io())
                    else {
                        continue;
                    };

                    let io = MonoioIo::new(stream);
                    let Ok((mut sender, conn)) = hyper::client::conn::http1::handshake(io).await
                    else {
                        continue;
                    };
                    monoio::spawn(conn);

                    loop {
                        if counter.fetch_add(1, Relaxed) < opt.n {
                            let req = Request::get(&url).body(Empty::<Bytes>::new()).unwrap();
                            let Ok(mut res) = sender.send_request(req).await else {
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

    let elapsed = now.elapsed();

    println!(
        "{} requests in {:.2?}, {:.2} req/s",
        opt.n,
        elapsed,
        opt.n as f64 / elapsed.as_secs_f64()
    );
}
