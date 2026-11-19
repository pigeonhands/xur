use md5::Md5;
use sha2::{Sha256, Digest};

use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::hash")
        .function("crc32", |data: Vec<u8>| crc32fast::hash(&data) as i64)
        .function("sha256", |data: Vec<u8>| {
            format!("{:x}", Sha256::digest(&data))
        })
        .function("sha256_bytes", |data: Vec<u8>| {
            Sha256::digest(&data).to_vec()
        })
        .function("md5", |data: Vec<u8>| {
            format!("{:x}", Md5::digest(&data))
        });
}
