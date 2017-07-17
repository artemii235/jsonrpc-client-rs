//! A crate for generating transport agnostic, auto serializing, strongly typed JSON-RPC 2.0 clients
//!
//! This crate mainly provides a macro, `jsonrpc_client`. The macro is be used to generate
//! structs for calling JSON-RPC 2.0 APIs. The macro lets you list methods on the struct with
//! arguments and a return value. The macro then generates a struct which will automatically
//! serialize the arguments, send the request and deserialize the response into the target type.
//!
//! # Example
//!
//! Look at the `ExampleRpcClient` struct in this crate. It uses the library to generate itself.
//!
//! Here is an example of how to generate and use a client struct:
//!
//! ```ignore
//! #[macro_use] extern crate jsonrpc_client_core;
//! extern crate jsonrpc_client_http;
//!
//! use jsonrpc_client_http::HttpTransport;
//!
//! jsonrpc_client!(pub struct FizzBuzzClient {
//!     /// Returns the fizz-buzz string for the given number.
//!     pub fn fizz_buzz(&mut self, number: u64) -> Result<String>;
//! });
//!
//! fn main() {
//!     let transport = HttpTransport::new("https://api.fizzbuzzexample.org/rpc/").unwrap();
//!     let mut client = FizzBuzzClient::new(transport);
//!     let result1 = client.fizz_buzz(3).unwrap();
//!     let result2 = client.fizz_buzz(4).unwrap();
//!     let result3 = client.fizz_buzz(5).unwrap();
//!     // Should print "fizz 4 buzz" if the server implemented the service correctly
//!     println!("{} {} {}", result1, result2, result3);
//! }
//! ```
//!

#[macro_use]
extern crate error_chain;
extern crate jsonrpc_core;
#[macro_use]
extern crate serde_json;
extern crate serde;
#[macro_use]
extern crate log;

error_chain! {
    errors {
        /// Error in the underlying transport layer.
        TransportError {
            description("Unable to send the JSON-RPC 2.0 request")
        }
        /// Error while serializing method parameters.
        SerializeError {
            description("Unable to serialize the method parameters")
        }
        /// Error while deserializing or parsing the response data.
        ResponseError(msg: &'static str) {
            description("Unable to deserialize the response into the desired type")
            display("Unable to deserialize the response: {}", msg)
        }
        JsonRpcError(error: jsonrpc_core::types::error::Error) {
            description("Method call returned JSON-RPC-2.0 error")
            display("JSON-RPC-2.0 Error: {} ({})", error.code.description(), error.message)
        }
    }
}

/// Trait for types acting as a transport layer for the JSON-RPC 2.0 clients generated by the
/// `jsonrpc_client` macro.
pub trait Transport<E: ::std::error::Error + Send + 'static> {
    fn send(&mut self, json_data: &[u8]) -> ::std::result::Result<Vec<u8>, E>;
}


/// The main macro of this crate. Generates JSON-RPC 2.0 client structs with automatic serialization
/// and deserialization. Method calls get correct types automatically.
#[macro_export]
macro_rules! jsonrpc_client {
    (
        $(#[$struct_doc:meta])*
        pub struct $struct_name:ident {$(
            $(#[$doc:meta])*
            pub fn $method:ident(&mut $selff:ident $(, $arg_name:ident: $arg_ty:ty)*)
                -> Result<$return_ty:ty>;
        )*}
    ) => (
        $(#[$struct_doc])*
        pub struct $struct_name<E, T>
            where E: ::std::error::Error + Send + 'static, T: $crate::Transport<E>
        {
            transport: T,
            id: u64,
            _error: ::std::marker::PhantomData<E>,
        }

        impl<E: ::std::error::Error + Send + 'static, T: $crate::Transport<E>> $struct_name<E, T> {
            /// Creates a new RPC client backed by the given transport implementation.
            pub fn new(transport: T) -> Self {
                $struct_name {
                    transport,
                    id: 0,
                    _error: ::std::marker::PhantomData,
                }
            }

            $(
                $(#[$doc])*
                pub fn $method(&mut $selff $(, $arg_name: $arg_ty)*) -> $crate::Result<$return_ty> {
                    $selff.id += 1;
                    let method = stringify!($method);
                    let params = ($($arg_name,)*);
                    $crate::call_method(&mut $selff.transport, $selff.id, method, params)
                }
            )*
        }
    )
}


/// Call a method with a given transport, method and parameters. Not intended for direct use.
/// Is being called from the client structs generated by the `jsonrpc_client` macro.
pub fn call_method<E, T, P, R>(transport: &mut T, id: u64, method: &str, params: P) -> Result<R>
where
    E: ::std::error::Error + Send + 'static,
    T: Transport<E>,
    P: serde::Serialize,
    for<'de> R: serde::Deserialize<'de>,
{
    let request_json = format_request(id, method, params);
    let request_raw = serde_json::to_vec(&request_json)
        .chain_err(|| ErrorKind::SerializeError)?;

    debug!("Sending JSON-RPC 2.0 request: {}", request_json);
    let response_raw = transport
        .send(&request_raw)
        .chain_err(|| ErrorKind::TransportError)?;

    parse_response::<R>(&response_raw, id)
}


/// Creates a JSON-RPC 2.0 request to the given method with the given parameters.
fn format_request<P>(id: u64, method: &str, params: P) -> serde_json::Value
where
    P: serde::Serialize,
{
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}


/// Parses a binary response into json, extracts the "result" field and tries to deserialize that
/// to the desired type.
fn parse_response<T>(response: &[u8], expected_id: u64) -> Result<T>
where
    for<'de> T: serde::Deserialize<'de>,
{
    let response_map = get_response_as_map(response)?;
    let result_json = check_response_and_get_result(response_map, expected_id)?;
    debug!("Received json result: {}", result_json);
    serde_json::from_value::<T>(result_json).chain_err(|| {
        ErrorKind::ResponseError("Result cannot deserialize to target type")
    })
}

fn get_response_as_map(response: &[u8]) -> Result<serde_json::Map<String, serde_json::Value>> {
    let response_json = serde_json::from_slice(response)
        .chain_err(|| ErrorKind::ResponseError("Response is not valid json"))?;
    if let serde_json::Value::Object(map) = response_json {
        Ok(map)
    } else {
        Err(
            ErrorKind::ResponseError("Response is not a json object").into(),
        )
    }
}

fn check_response_and_get_result(
    mut response_map: serde_json::Map<String, serde_json::Value>,
    expected_id: u64,
) -> Result<serde_json::Value> {
    ensure!(
        response_map.remove("jsonrpc") == Some(serde_json::Value::String("2.0".to_owned())),
        ErrorKind::ResponseError("Response is not JSON-RPC 2.0 compatible")
    );
    ensure!(
        response_map.remove("id") == Some(expected_id.into()),
        ErrorKind::ResponseError("Response id not equal to request id")
    );
    if let Some(error_json) = response_map.remove("error") {
        let error = json_value_to_rpc_error(error_json)
            .chain_err(|| ErrorKind::ResponseError("Malformed error object"))?;
        bail!(ErrorKind::JsonRpcError(error));
    }
    response_map.remove("result").ok_or(
        ErrorKind::ResponseError("Response has no \"result\" field").into(),
    )
}

fn json_value_to_rpc_error(
    mut error_json: serde_json::Value,
) -> Result<jsonrpc_core::types::error::Error> {
    let map = error_json
        .as_object_mut()
        .ok_or(ErrorKind::ResponseError("Error is not a json object"))?;
    let code = map.remove("code")
        .ok_or(ErrorKind::ResponseError("Error has no code field").into())
        .and_then(|code| {
            serde_json::from_value(code)
                .chain_err(|| ErrorKind::ResponseError("Malformed code field in error"))
        })?;
    let message = map.get("message")
        .and_then(|v| v.as_str())
        .ok_or(ErrorKind::ResponseError(
            "Error has no message field of string type",
        ))?
        .to_owned();

    Ok(jsonrpc_core::types::error::Error {
        code: code,
        message: message,
        data: map.remove("data"),
    })
}



jsonrpc_client!(
    /// Just an example RPC client to showcase how to use the `jsonrpc_client` macro and what
    /// the resulting structs look like.
    pub struct ExampleRpcClient {
        /// A method without any arguments and with a null return value. Can still of course have
        /// lots of side effects on the server where it executes.
        pub fn nullary(&mut self) -> Result<()>;

        pub fn echo(&mut self, input: String) -> Result<String>;

        /// Example RPC method named "concat" that takes a `String` and an unsigned integer and
        /// returns a `String`. From the name one could guess it will concatenate the two
        /// arguments. But that of course depends on the server where this call is sent.
        pub fn concat(&mut self, arg0: String, arg1: u64) -> Result<String>;
    }
);



#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    /// A test transport that just echoes back a response containing the request as the result.
    struct EchoTransport;

    impl Transport<io::Error> for EchoTransport {
        fn send(&mut self, json_data: &[u8]) -> ::std::result::Result<Vec<u8>, io::Error> {
            let json = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": serde_json::from_slice::<serde_json::Value>(json_data).unwrap(),
            });
            Ok(serde_json::to_vec(&json).unwrap())
        }
    }

    /// A transport that always returns an "Invalid request" error
    struct ErrorTransport;

    impl Transport<io::Error> for ErrorTransport {
        fn send(&mut self, _json_data: &[u8]) -> ::std::result::Result<Vec<u8>, io::Error> {
            let json = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {
                    "code": -32600,
                    "message": "This was an invalid request",
                    "data": [1, 2, 3],
                }
            });
            Ok(serde_json::to_vec(&json).unwrap())
        }
    }

    jsonrpc_client!(pub struct TestRpcClient {
        pub fn ping(&mut self, arg0: String) -> Result<serde_json::Value>;
    });

    #[test]
    fn echo() {
        let mut client = TestRpcClient::new(EchoTransport);
        let result = client.ping("Hello".to_string()).unwrap();
        if let serde_json::Value::Object(mut map) = result {
            assert_eq!(Some(serde_json::Value::String("2.0".to_string())), map.remove("jsonrpc"));
            assert_eq!(Some(serde_json::Value::Number(1.into())), map.remove("id"));
            assert_eq!(Some(serde_json::Value::String("ping".to_string())), map.remove("method"));
            assert_eq!(Some(serde_json::Value::Array(vec!["Hello".into()])), map.remove("params"));
        } else {
            panic!("Invalid response type: {:?}", result);
        }
    }

    #[test]
    fn error() {
        let mut client = TestRpcClient::new(ErrorTransport);
        let error = client.ping("".to_string()).unwrap_err();
        if let &ErrorKind::JsonRpcError(ref json_error) = error.kind() {
            use jsonrpc_core::types::error::ErrorCode;
            assert_eq!(ErrorCode::InvalidRequest, json_error.code);
            assert_eq!("This was an invalid request", json_error.message);
            assert_eq!(Some(json!{[1, 2, 3]}), json_error.data);
        } else {
            panic!("Wrong error kind");
        }
    }
}
