use hyper::http;
use hyper::server::conn::Http;
use hyper::service::service_fn;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use std::io;

const HTTP_ADDR: &str = "127.0.0.1:4001";

fn main() {
  tokio::runtime::Builder::new_current_thread()
    .enable_io()
    .enable_time()
    .build()
    .unwrap()
    .block_on(async_main())
    .unwrap()
}

async fn async_main() -> io::Result<()> {
  let server = tokio::net::TcpListener::bind(HTTP_ADDR).await?;
  println!("HTTP address: {}", server.local_addr().unwrap());

  loop {
    let (client, _addr) = server.accept().await?;
    tokio::spawn(handle_client(client));
  }
}

async fn handle_client(client: tokio::net::TcpStream) -> hyper::Result<()> {
  let handler = move |req| handle_request(req);
  let handler = service_fn(handler);
  Http::new().serve_connection(client, handler).await
}

async fn handle_request(_req: Request<Body>) -> http::Result<Response<Body>> {
  let res = Response::builder()
    .status(200)
    .header(hyper::header::CONTENT_LENGTH, "2")
    .header(hyper::header::DATE, "Wed, 27 Jan 2021 10:55:19 GMT")
    .body("ok".into())
    .unwrap();
  Ok(res)
}
