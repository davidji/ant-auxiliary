#![no_std]
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Set {
    pub lights: bool,
    pub fan: u16
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Query {

}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum RequestBody {
    Query, Set
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Request {
    // pub message_id : u64,
    pub correlation_id : i32,
    pub body : RequestBody
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Fan {
    pub power: u16,
    pub rate:u16
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Temp {
    pub temp: i16,
    pub humidity: i16
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Status {
    pub temp: Temp,
    pub fan: Fan
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum ResponseBody {
    Status
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    pub correlation_id : i32,
    pub body : ResponseBody
}
