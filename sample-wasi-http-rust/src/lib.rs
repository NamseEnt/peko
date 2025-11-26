use wstd::http::body::{BodyForthcoming, IncomingBody};
use wstd::http::server::{Finished, Responder};
use wstd::http::{IntoBody, Request, Response, StatusCode};
use wstd::io::{copy, empty, AsyncWrite};
use wstd::time::{Duration, Instant};

#[wstd::http_server]
async fn main(req: Request<IncomingBody>, res: Responder) -> Finished {
    let path_and_query = req.uri().path_and_query().unwrap().as_str();

    // Extract path without query params for matching
    let path = path_and_query.split('?').next().unwrap();

    match path {
        "/wait" => wait(req, res).await,
        "/echo" => echo(req, res).await,
        "/echo-headers" => echo_headers(req, res).await,
        "/echo-trailers" => echo_trailers(req, res).await,
        "/trap" => trap(req, res).await,
        "/slow" => slow(req, res).await,
        "/error" => error_response(req, res).await,
        "/infinite-loop" => infinite_loop(req, res).await,
        "/alloc-memory" => alloc_memory(req, res).await,
        "/" => home(req, res).await,
        _ => not_found(req, res).await,
    }
}

async fn home(_req: Request<IncomingBody>, res: Responder) -> Finished {
    res.respond(Response::new("Hello, wasi:http/proxy world!\n".into_body()))
        .await
}

async fn wait(_req: Request<IncomingBody>, res: Responder) -> Finished {
    let now = Instant::now();
    wstd::task::sleep(Duration::from_secs(1)).await;
    let elapsed = Instant::now().duration_since(now).as_millis();

    let mut body = res.start_response(Response::new(BodyForthcoming));
    let result = body
        .write_all(format!("slept for {elapsed} millis\n").as_bytes())
        .await;
    Finished::finish(body, result, None)
}

async fn echo(mut req: Request<IncomingBody>, res: Responder) -> Finished {
    let mut body = res.start_response(Response::new(BodyForthcoming));
    let result = copy(req.body_mut(), &mut body).await;
    Finished::finish(body, result, None)
}

async fn echo_headers(req: Request<IncomingBody>, responder: Responder) -> Finished {
    let mut res = Response::builder();
    *res.headers_mut().unwrap() = req.into_parts().0.headers;
    let res = res.body(empty()).unwrap();
    responder.respond(res).await
}

async fn echo_trailers(req: Request<IncomingBody>, res: Responder) -> Finished {
    let body = res.start_response(Response::new(BodyForthcoming));
    let (trailers, result) = match req.into_body().finish().await {
        Ok(trailers) => (trailers, Ok(())),
        Err(err) => (Default::default(), Err(std::io::Error::other(err))),
    };
    Finished::finish(body, result, trailers)
}

async fn not_found(_req: Request<IncomingBody>, responder: Responder) -> Finished {
    let res = Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(empty())
        .unwrap();
    responder.respond(res).await
}

async fn trap(_req: Request<IncomingBody>, _res: Responder) -> Finished {
    panic!("Intentional trap for testing");
}

async fn slow(req: Request<IncomingBody>, res: Responder) -> Finished {
    let path_and_query = req.uri().path_and_query().unwrap().as_str();
    let mut sleep_ms: u64 = 100;

    if let Some(query) = path_and_query.split('?').nth(1) {
        for param in query.split('&') {
            if let Some((key, value)) = param.split_once('=') {
                if key == "ms" {
                    if let Ok(ms) = value.parse::<u64>() {
                        sleep_ms = ms;
                    }
                }
            }
        }
    }

    let now = Instant::now();
    wstd::task::sleep(Duration::from_millis(sleep_ms)).await;
    let elapsed = Instant::now().duration_since(now).as_millis();

    let mut body = res.start_response(Response::new(BodyForthcoming));
    let result = body
        .write_all(format!("slept for {elapsed} millis (requested {sleep_ms} ms)\n").as_bytes())
        .await;
    Finished::finish(body, result, None)
}

async fn error_response(_req: Request<IncomingBody>, responder: Responder) -> Finished {
    let res = Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body("Error endpoint - returns 500".into_body())
        .unwrap();
    responder.respond(res).await
}

async fn infinite_loop(_req: Request<IncomingBody>, _res: Responder) -> Finished {
    let mut counter: u64 = 0;
    loop {
        counter = counter.wrapping_add(1);
        std::hint::black_box(counter);
    }
}

async fn alloc_memory(_req: Request<IncomingBody>, res: Responder) -> Finished {
    // Allocate 150MB of memory (>128MB as requested)
    const MEMORY_SIZE: usize = 150 * 1024 * 1024; // 150MB in bytes

    let mut data: Vec<u8> = Vec::with_capacity(MEMORY_SIZE);

    // Fill the vector to actually allocate the memory
    for i in 0..MEMORY_SIZE {
        data.push((i % 256) as u8);
    }

    // Do some operation to prevent optimizer from removing the allocation
    let checksum: u64 = data.iter().map(|&x| x as u64).sum();
    let allocated_mb = data.len() / (1024 * 1024);

    let response_text = format!(
        "Successfully allocated {} MB of memory\nChecksum: {}\nCapacity: {} bytes\n",
        allocated_mb,
        checksum,
        data.capacity()
    );

    res.respond(Response::new(response_text.into_body())).await
}
