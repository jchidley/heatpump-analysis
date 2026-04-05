//! eBUS protocol primitives — CRC, byte stuffing, address classification,
//! telegram parsing, and SYN-delimited framing.
//!
//! Ported from [yuhu-ebus](https://github.com/yuhu-ebus/ebus) (Roland Jax,
//! GPL-3.0). This crate is an independent `no_std` reimplementation of the
//! protocol-layer logic only, licensed MIT/Apache-2.0.
//!
//! # Modules
//!
//! * [`symbols`] — protocol constants (SYN, ACK, NAK, escape bytes)
//! * [`crc`] — CRC-8 table and calculation (polynomial 0x9B)
//! * [`address`] — master/slave address classification
//! * [`sequence`] — byte stuffing (extend) and unstuffing (reduce)
//! * [`telegram`] — telegram parsing with master + slave parts
//! * [`framer`] — SYN-delimited byte stream → complete telegrams

#![cfg_attr(not(feature = "std"), no_std)]

pub mod address;
pub mod crc;
pub mod framer;
pub mod sequence;
pub mod symbols;
pub mod telegram;
