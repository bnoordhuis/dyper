use hyper::http;
use hyper::server::conn::Http;
use hyper::service::service_fn;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use std::cell::RefCell;
use std::convert::TryFrom;
use std::env;
use std::fs;
use std::io;
use std::process;

const HTTP_ADDR: &str = "127.0.0.1:4000";

thread_local! {
  static RUNTIME: RefCell<Option<Runtime>> = RefCell::new(None);
}

struct Runtime {
  isolate: v8::OwnedIsolate,
  context: v8::Global<v8::Context>,
  callback: v8::Global<v8::Function>,
}

impl Runtime {
  fn borrow_mut<F, T>(mut f: F) -> T
  where
    F: FnMut(&mut Runtime) -> T,
  {
    RUNTIME.with(|slot| f(slot.borrow_mut().as_mut().unwrap()))
  }
}

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
  let mut rest = vec![];

  for arg in env::args().skip(1) {
    match arg {
      arg if !arg.starts_with('-') => rest.push(arg),
      arg if !rest.is_empty() => rest.push(arg),
      arg => v8::V8::set_flags_from_string(&arg),
    }
  }

  if rest.is_empty() {
    eprintln!("Tell me what file to load.");
    process::exit(1);
  }

  v8::V8::initialize_platform(v8::new_default_platform().unwrap());
  v8::V8::initialize();

  let params = v8::CreateParams::default().heap_limits(0, 128 << 20);
  let mut isolate = v8::Isolate::new(params);

  let (context, callback) = {
    let scope = &mut v8::HandleScope::new(&mut isolate);
    let context = v8::Context::new(scope);

    let scope = &mut v8::ContextScope::new(scope, context);
    let scope = &mut v8::TryCatch::new(scope);
    let global = context.global(scope);

    // TODO(bnoordhuis) Use snapshot.
    let filename = concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.js");
    let source = fs::read_to_string(filename).unwrap();
    let result = execute_script(scope, &filename, &source);

    // Patch console.log(). Overriding the built-in console object with
    // Context::new_from_template() doesn't work because of a check in V8.
    let name = v8_string(scope, "console");
    let console = global.get(scope, name.into()).unwrap();
    let console = v8::Local::<v8::Object>::try_from(console).unwrap();
    let log = v8::Function::new(scope, console_log_callback).unwrap();
    set_named(scope, console, "log", log);

    let function = v8::Local::<v8::Function>::try_from(result)
      .expect("vm entry point is not a function");
    let undefined = v8::undefined(scope).into();
    let api = v8::Function::new(scope, api_callback).unwrap();
    let exports = v8::Object::new(scope);
    let result = function
      .call(scope, undefined, &[api.into(), exports.into()])
      .unwrap_or_else(|| print_stack_trace_and_exit(scope));

    let callback = v8::Local::<v8::Function>::try_from(result)
      .expect("vm entry point did not return a function");
    let callback = v8::Global::new(scope, callback);

    set_named(scope, global, "dyper", exports);

    let filename = rest.first().unwrap();
    let source = std::fs::read_to_string(filename)
      .expect(&format!("file not found: {}", filename));

    execute_script(scope, &filename, &source);

    let context = v8::Global::new(scope, context);
    (context, callback)
  };

  let runtime = Runtime {
    isolate,
    context,
    callback,
  };

  RUNTIME.with(|slot| *slot.borrow_mut() = Some(runtime));

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

async fn handle_request(req: Request<Body>) -> http::Result<Response<Body>> {
  Runtime::borrow_mut(|runtime| {
    let Runtime {
      isolate,
      context,
      callback,
    } = runtime;

    let scope = &mut v8::HandleScope::with_context(isolate, context.clone());
    let scope = &mut v8::TryCatch::new(scope);

    let callback = v8::Local::new(scope, callback.clone());
    let undefined = v8::undefined(scope);

    let mut headers = vec![];

    for (name, value) in req.headers().iter() {
      let name = name.to_string();
      let name = v8_string(scope, &name);
      headers.push(name);

      let value = value.to_str().unwrap_or("");
      let value = v8_string(scope, &value);
      headers.push(value);
    }

    let headers = v8_array(scope, headers);
    let method = v8_string(scope, req.method().as_str());
    let uri = v8_string(scope, &req.uri().to_string());

    let args = &[method.into(), uri.into(), headers.into()];

    match callback.call(scope, undefined.into(), args) {
      None => {
        print_stack_trace(scope);
        Response::builder()
          .status(500)
          .body("internal server error".into())
      }
      Some(result) => {
        let result = v8::Local::<v8::Array>::try_from(result)
          .expect("return value: array expected");

        assert_eq!(3, result.length());

        let status = result
          .get_index(scope, 0)
          .unwrap()
          .uint32_value(scope)
          .unwrap();

        let headers = result.get_index(scope, 1).unwrap();

        let headers = v8::Local::<v8::Array>::try_from(headers)
          .expect("headers element: array expected");

        let body = result
          .get_index(scope, 2)
          .unwrap()
          .to_rust_string_lossy(scope);

        let mut res = Response::builder().status(status as u16);
        let map = res.headers_mut().unwrap();

        let mut index = 0;
        let length = headers.length();

        while index + 1 < length {
          let name = headers
            .get_index(scope, index)
            .unwrap()
            .to_rust_string_lossy(scope);

          let name =
            hyper::header::HeaderName::from_bytes(name.as_bytes()).unwrap();

          let value = headers
            .get_index(scope, index + 1)
            .unwrap()
            .to_rust_string_lossy(scope);

          let value = http::HeaderValue::from_str(&value).unwrap();

          map.append(name, value);

          index += 2;
        }

        res.body(body.into())
      }
    }
  })
}

fn api_callback(
  _scope: &mut v8::HandleScope,
  _args: v8::FunctionCallbackArguments,
  _: v8::ReturnValue,
) {
  todo!()
}

fn console_log_callback(
  scope: &mut v8::HandleScope,
  args: v8::FunctionCallbackArguments,
  _: v8::ReturnValue,
) {
  let string = (0..args.length())
    .map(|index| args.get(index).to_rust_string_lossy(scope))
    .collect::<Vec<_>>()
    .join(" ");

  let empty = v8::String::empty(scope);
  let exception = v8::Exception::error(scope, empty);
  let message = v8::Exception::create_message(scope, exception);
  let script_name = message
    .get_script_resource_name(scope)
    .map(|name| name.to_rust_string_lossy(scope))
    .unwrap_or_else(|| "<unknown>".to_string());
  let line_number = message.get_line_number(scope).unwrap_or(0);

  println!("[info {}:{}] {}", script_name, line_number, string);
}

fn execute_script<'s>(
  scope: &mut v8::HandleScope<'s>,
  name: &str,
  source: &str,
) -> v8::Local<'s, v8::Value> {
  let scope = &mut v8::TryCatch::new(scope);
  let source = v8_string(scope, source);
  let origin = script_origin(scope, name);

  v8::Script::compile(scope, source, Some(&origin))
    .and_then(|script| script.run(scope))
    .or_else(|| print_stack_trace_and_exit(scope))
    .unwrap_or_else(|| print_stack_trace_and_exit(scope))
}

fn script_origin<'s>(
  scope: &mut v8::HandleScope<'s>,
  name: &str,
) -> v8::ScriptOrigin<'s> {
  let name = v8_string(scope, name);
  let empty = v8::String::empty(scope);

  v8::ScriptOrigin::new(
    scope,
    name.into(),  // resource_name
    0,            // resource_line_offset
    0,            // resource_column_offset
    true,         // resource_is_shared_cross_origin - I guess it is?
    0,            // script_id
    empty.into(), // source_map_url
    true,         // resource_is_opaque
    false,        // is_wasm
    false,        // is_module
  )
}

fn print_stack_trace_and_exit(scope: &mut v8::TryCatch<v8::HandleScope>) -> ! {
  print_stack_trace(scope);
  process::exit(1);
}

fn print_stack_trace(scope: &mut v8::TryCatch<v8::HandleScope>) {
  eprintln!("{}", stack_trace(scope));
}

fn stack_trace(scope: &mut v8::TryCatch<v8::HandleScope>) -> String {
  None
    .or_else(|| scope.stack_trace())
    .or_else(|| scope.exception())
    .map(|value| value.to_rust_string_lossy(scope))
    .unwrap_or_else(|| "no exception".to_string())
}

fn set_named<'s, R, V>(
  scope: &mut v8::TryCatch<v8::HandleScope<'s>>,
  recv: R,
  name: &str,
  value: V,
) where
  R: Into<v8::Local<'s, v8::Object>>,
  V: Into<v8::Local<'s, v8::Value>>,
{
  let name = v8_string(scope, name);
  assert!(recv
    .into()
    .set(scope, name.into(), value.into())
    .unwrap_or_else(|| print_stack_trace_and_exit(scope)));
}

fn v8_string<'s>(
  scope: &mut v8::HandleScope<'s>,
  string: &str,
) -> v8::Local<'s, v8::String> {
  v8::String::new(scope, string).unwrap()
}

fn v8_array<'s, T>(
  scope: &mut v8::HandleScope<'s>,
  items: Vec<T>,
) -> v8::Local<'s, v8::Array>
where
  T: Into<v8::Local<'s, v8::Value>>,
{
  let array = v8::Array::new(scope, items.len() as i32);

  let mut index = 0;
  for item in items {
    array.set_index(scope, index, item.into());
    index += 1;
  }

  array
}
