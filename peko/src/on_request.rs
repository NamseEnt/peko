use hyper::body::Incoming;
use tokio::sync::mpsc::Sender;

use crate::*;

type Req = hyper::Request<Incoming>;
type Res = hyper::Response<Incoming>;

pub enum Error {
    AllWorkerBusy,
}

type Result<T> = std::result::Result<T, Error>;

pub async fn on_request(req: Req, res: Res, execute_tx: Sender<execute::Job>) -> Result<()> {
    for _ in 0..2 {
        match find_proper_worker().await? {
            FindWorkerResult::Me => return execute_on_me(req, res, execute_tx).await,
            FindWorkerResult::Other { address } => {
                return execute_on_other(address, req, res).await;
            }
            FindWorkerResult::Nobody => {
                tokio::time::sleep(tokio::time::Duration::from_millis(14)).await;
            }
        }
    }

    Err(Error::AllWorkerBusy)
}

async fn execute_on_me(req: Req, res: Res, execute_tx: Sender<execute::Job>) -> Result<()> {
    execute_tx
        .send(execute::Job {
            req,
            res,
            code_id: "todo".to_string(),
            fn_name: "todo".to_string(),
        })
        .await
        .unwrap();
    todo!()
}

async fn execute_on_other(address: Address, req: Req, res: Res) -> Result<()> {
    todo!()
}

enum FindWorkerResult {
    Me,
    Other { address: Address },
    Nobody,
}

struct Address {}

async fn find_proper_worker() -> Result<FindWorkerResult> {
    todo!()
}
