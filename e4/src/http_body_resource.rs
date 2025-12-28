use bytes::Bytes;
use deno_core::*;
use deno_error::JsErrorBox;
use futures::StreamExt;
use std::borrow::Cow;
use std::pin::Pin;
use std::rc::Rc;

type DynHttpStream = Pin<Box<dyn futures::Stream<Item = std::io::Result<Bytes>> + Send + Sync>>;

pub struct HttpBodyResource {
    pub stream: Rc<AsyncRefCell<Option<DynHttpStream>>>,
    cancel: Rc<CancelHandle>,
}

impl HttpBodyResource {
    pub fn new<B>(body: B) -> Self
    where
        B: hyper::body::Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
    {
        let body_stream = http_body_util::BodyStream::new(body);
        let stream = body_stream.map(|res| {
            res.map(|frame| frame.into_data().unwrap_or_default()) // Frame -> Bytes
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.into()))
        });

        Self {
            stream: Rc::new(AsyncRefCell::new(Some(Box::pin(stream)))),
            cancel: CancelHandle::new_rc(),
        }
    }
}

impl Resource for HttpBodyResource {
    fn name(&self) -> Cow<'_, str> {
        "httpBody".into()
    }

    fn close(self: Rc<Self>) {
        self.cancel.cancel();
    }

    fn read(self: Rc<Self>, _limit: usize) -> AsyncResult<BufView> {
        let cancel = self.cancel.clone();
        Box::pin(
            async move {
                let mut stream_guard = self.stream.borrow_mut().await;
                if let Some(ref mut stream) = *stream_guard {
                    match stream.next().await {
                        Some(Ok(bytes)) => Ok(BufView::from(bytes)),
                        Some(Err(e)) => Err(JsErrorBox::from_err(e)),
                        None => Ok(BufView::empty()),
                    }
                } else {
                    Err(JsErrorBox::generic("Stream already taken"))
                }
            }
            .try_or_cancel(cancel),
        )
    }
}
