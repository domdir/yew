mod private;
mod public;

pub use private::Private;
pub use public::Public;

use super::*;
use cfg_if::cfg_if;
use cfg_match::cfg_match;
use serde::{Deserialize, Serialize};
cfg_if! {
    if #[cfg(feature = "std_web")] {
        use stdweb::Value;
        #[allow(unused_imports)]
        use stdweb::{_js_impl, js};
    } else if #[cfg(feature = "web_sys")] {
        use crate::utils;
        use js_sys::{Array, Reflect, Uint8Array};
        use wasm_bindgen::{closure::Closure, JsCast, JsValue};
        use web_sys::{Blob, BlobPropertyBag, DedicatedWorkerGlobalScope, MessageEvent, Url, Worker, WorkerOptions};
    }
}

/// Implements rules to register a worker in a separate thread.
pub trait Threaded {
    /// Executes an agent in the current environment.
    /// Uses in `main` function of a worker.
    fn register();
}

/// Message packager, based on serde::Serialize/Deserialize
pub trait Packed {
    /// Pack serializable message into Vec<u8>
    fn pack(&self) -> Vec<u8>;
    /// Unpack deserializable message of byte slice
    fn unpack(data: &[u8]) -> Self;
}

impl<T: Serialize + for<'de> Deserialize<'de>> Packed for T {
    fn pack(&self) -> Vec<u8> {
        bincode::serialize(&self).expect("can't serialize an agent message")
    }

    fn unpack(data: &[u8]) -> Self {
        bincode::deserialize(&data).expect("can't deserialize an agent message")
    }
}

/// Serializable messages to worker
#[derive(Serialize, Deserialize, Debug)]
enum ToWorker<T> {
    /// Client is connected
    Connected(HandlerId),
    /// Incoming message to Worker
    ProcessInput(HandlerId, T),
    /// Client is disconnected
    Disconnected(HandlerId),
    /// Worker should be terminated
    Destroy,
}

/// Serializable messages sent by worker to consumer
#[derive(Serialize, Deserialize, Debug)]
enum FromWorker<T> {
    /// Worker sends this message when `wasm` bundle has loaded.
    WorkerLoaded,
    /// Outgoing message to consumer
    ProcessOutput(HandlerId, T),
}

fn send_to_remote<AGN: Agent>(
    #[cfg(feature = "std_web")] worker: &Value,
    #[cfg(feature = "web_sys")] worker: &Worker,
    msg: ToWorker<AGN::Input>,
) {
    let msg = msg.pack();
    cfg_match! {
        feature = "std_web" => js! {
            var worker = @{worker};
            var bytes = @{msg};
            worker.postMessage(bytes);
        },
        feature = "web_sys" => worker.post_message_vec(msg),
    };
}

#[cfg(feature = "web_sys")]
fn worker_new(name_of_resource: &str, is_module: bool) -> Worker {
    let href = utils::document().location().unwrap().href().unwrap();
    let script_url = format!("{}{}", href, name_of_resource);
    let wasm_url = format!("{}{}", href, name_of_resource.replace(".js", "_bg.wasm"));
    let array = Array::new();
    array.push(
        &format!(
            r#"importScripts("{}");wasm_bindgen("{}");"#,
            script_url, wasm_url
        )
        .into(),
    );
    let blob = Blob::new_with_str_sequence_and_options(
        &array,
        BlobPropertyBag::new().type_("application/javascript"),
    )
    .unwrap();
    let url = Url::create_object_url_with_blob(&blob).unwrap();

    if is_module {
        let options = WorkerOptions::new();
        Reflect::set(
            options.as_ref(),
            &JsValue::from_str("type"),
            &JsValue::from_str("module"),
        )
        .unwrap();
        Worker::new_with_options(&url, &options).expect("failed to spawn worker")
    } else {
        Worker::new(&url).expect("failed to spawn worker")
    }
}

#[cfg(feature = "web_sys")]
fn worker_self() -> DedicatedWorkerGlobalScope {
    JsValue::from(js_sys::global()).into()
}

#[cfg(feature = "web_sys")]
trait WorkerExt {
    fn set_onmessage_closure(&self, handler: impl 'static + Fn(Vec<u8>));

    fn post_message_vec(&self, data: Vec<u8>);
}

#[cfg(feature = "web_sys")]
macro_rules! worker_ext_impl {
    ($($type:ident),+) => {$(
        impl WorkerExt for $type {
            fn set_onmessage_closure(&self, handler: impl 'static + Fn(Vec<u8>)) {
                let handler = move |message: MessageEvent| {
                    let data = Uint8Array::from(message.data()).to_vec();
                    handler(data);
                };
                let closure = Closure::wrap(Box::new(handler) as Box<dyn Fn(MessageEvent)>);
                self.set_onmessage(Some(closure.as_ref().unchecked_ref()));
                closure.forget();
            }

            fn post_message_vec(&self, data: Vec<u8>) {
                self.post_message(&Uint8Array::from(data.as_slice()))
                    .expect("failed to post message");
            }
        }
    )+};
}

#[cfg(feature = "web_sys")]
worker_ext_impl! {
    Worker, DedicatedWorkerGlobalScope
}
