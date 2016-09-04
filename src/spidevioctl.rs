// Copyright 2015, Paul Osborne <osbpau@gmail.com>
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/license/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option.  This file may not be copied, modified, or distributed
// except according to those terms.

#![allow(dead_code)]

use std::io;
use std::mem;
use std::os::unix::prelude::*;
use super::SpiModeFlags;

fn from_nix_error(err: ::nix::Error) -> io::Error {
    io::Error::from_raw_os_error(err.errno() as i32)
}

fn from_nix_result<T>(res: ::nix::Result<T>) -> io::Result<T> {
    match res {
        Ok(r) => Ok(r),
        Err(err) => Err(from_nix_error(err)),
    }
}


/// Structure that is used when performing communication
/// with the kernel.
///
/// From the kernel documentation:
///
/// ```text
/// struct spi_ioc_transfer - describes a single SPI transfer
/// @tx_buf: Holds pointer to userspace buffer with transmit data, or null.
///   If no data is provided, zeroes are shifted out.
/// @rx_buf: Holds pointer to userspace buffer for receive data, or null.
/// @len: Length of tx and rx buffers, in bytes.
/// @speed_hz: Temporary override of the device's bitrate.
/// @bits_per_word: Temporary override of the device's wordsize.
/// @delay_usecs: If nonzero, how long to delay after the last bit transfer
///      before optionally deselecting the device before the next transfer.
/// @cs_change: True to deselect device before starting the next transfer.
///
/// This structure is mapped directly to the kernel spi_transfer structure;
/// the fields have the same meanings, except of course that the pointers
/// are in a different address space (and may be of different sizes in some
/// cases, such as 32-bit i386 userspace over a 64-bit x86_64 kernel).
/// Zero-initialize the structure, including currently unused fields, to
/// accommodate potential future updates.
///
/// SPI_IOC_MESSAGE gives userspace the equivalent of kernel spi_sync().
/// Pass it an array of related transfers, they'll execute together.
/// Each transfer may be half duplex (either direction) or full duplex.
///
///      struct spi_ioc_transfer mesg[4];
///      ...
///      status = ioctl(fd, SPI_IOC_MESSAGE(4), mesg);
///
/// So for example one transfer might send a nine bit command (right aligned
/// in a 16-bit word), the next could read a block of 8-bit data before
/// terminating that command by temporarily deselecting the chip; the next
/// could send a different nine bit command (re-selecting the chip), and the
/// last transfer might write some register values.
/// ```
#[allow(non_camel_case_types)]
#[derive(Debug)]
#[repr(C)]
pub struct spi_ioc_transfer {
    pub tx_buf: u64,
    pub rx_buf: u64,
    pub len: u32,

    // optional overrides
    pub speed_hz: u32,
    pub delay_usecs: u16,
    pub bits_per_word: u8,
    pub cs_change: u8,
    pub pad: u32,
}

mod ioctl {
    use super::spi_ioc_transfer;

    const SPI_IOC_MAGIC: u8 = 'k' as u8;
    const SPI_IOC_NR_TRANSFER: u8 = 0;
    const SPI_IOC_NR_MODE: u8 = 1;
    const SPI_IOC_NR_LSB_FIRST: u8 = 2;
    const SPI_IOC_NR_BITS_PER_WORD: u8 = 3;
    const SPI_IOC_NR_MAX_SPEED_HZ: u8 = 4;
    const SPI_IOC_NR_MODE32: u8 = 5;

    ioctl!(read get_mode_u8 with SPI_IOC_MAGIC, SPI_IOC_NR_MODE; u8);
    ioctl!(read get_mode_u32 with SPI_IOC_MAGIC, SPI_IOC_NR_MODE; u32);
    ioctl!(write set_mode_u8 with SPI_IOC_MAGIC, SPI_IOC_NR_MODE; u8);
    ioctl!(write set_mode_u32 with SPI_IOC_MAGIC, SPI_IOC_NR_MODE32; u32);

    ioctl!(read  get_lsb_first with SPI_IOC_MAGIC, SPI_IOC_NR_LSB_FIRST; u8);
    ioctl!(write set_lsb_first with SPI_IOC_MAGIC, SPI_IOC_NR_LSB_FIRST; u8);

    ioctl!(read  get_bits_per_word with SPI_IOC_MAGIC, SPI_IOC_NR_BITS_PER_WORD; u8);
    ioctl!(write set_bits_per_word with SPI_IOC_MAGIC, SPI_IOC_NR_BITS_PER_WORD; u8);

    ioctl!(read  get_max_speed_hz with SPI_IOC_MAGIC, SPI_IOC_NR_MAX_SPEED_HZ; u32);
    ioctl!(write set_max_speed_hz with SPI_IOC_MAGIC, SPI_IOC_NR_MAX_SPEED_HZ; u32);

    // NOTE: this macro works for single transfers but cannot properly
    // calculate size for multi transfer whose length we will not know
    // until runtime.  We fallback to using the underlying ioctl for that
    // use case.
    ioctl!(write spidev_transfer with SPI_IOC_MAGIC, SPI_IOC_NR_TRANSFER; spi_ioc_transfer);
    ioctl!(write buf spidev_transfer_buf with SPI_IOC_MAGIC, SPI_IOC_NR_TRANSFER; spi_ioc_transfer);
}

/// Representation of a spidev transfer that is shared
/// with external users
#[derive(Default)]
pub struct SpidevTransfer {
    pub tx_buf: Option<Box<[u8]>>,
    pub rx_buf: Option<Box<[u8]>>,
    len: u32,
    speed_hz: u32,
    delay_usecs: u16,
    bits_per_word: u8,
    cs_change: u8,
    pad: u32,
}

impl SpidevTransfer {
    pub fn read(length: usize) -> SpidevTransfer {
        SpidevTransfer {
            tx_buf: None,
            rx_buf: Some(vec![0u8; length].into_boxed_slice()),
            len: length as u32,
            ..Default::default()
        }
    }

    pub fn write(tx_buf: &[u8]) -> SpidevTransfer {
        let len = tx_buf.len();
        let rx_buf_vec: Vec<u8> = vec![0; len];
        let mut tx_buf_vec = Vec::with_capacity(len);
        for i in 0..len {
            tx_buf_vec.push(tx_buf[i]);
        }

        SpidevTransfer {
            tx_buf: Some(tx_buf_vec.into_boxed_slice()),
            rx_buf: Some(rx_buf_vec.into_boxed_slice()),
            len: tx_buf.len() as u32,
            ..Default::default()
        }
    }

    pub fn cs_change(mut self, cs_change: bool) -> Self {
        self.cs_change = if cs_change { 1 } else { 0 };
        self
    }

    fn as_spi_ioc_transfer(&self) -> spi_ioc_transfer {
        spi_ioc_transfer {
            tx_buf: match self.tx_buf {
                Some(ref bufbox) => bufbox.as_ptr() as u64,
                None => 0,
            },
            rx_buf: match self.rx_buf {
                Some(ref bufbox) => bufbox.as_ptr() as u64,
                None => 0,
            },
            len: self.len,
            speed_hz: self.speed_hz,
            delay_usecs: self.delay_usecs,
            bits_per_word: self.bits_per_word,
            cs_change: self.cs_change,
            pad: self.pad,
        }
    }
}


pub fn get_mode(fd: RawFd) -> io::Result<u8> {
    let mut mode: u8 = 0;
    try!(from_nix_result(unsafe { ioctl::get_mode_u8(fd, &mut mode) }));
    Ok(mode)
}

pub fn set_mode(fd: RawFd, mode: SpiModeFlags) -> io::Result<()> {
    // we will always use the 8-bit mode write unless bits not in
    // the 8-bit mask are used.  This is because WR_MODE32 was not
    // added until later kernels.  This provides a reasonable story
    // for forwards and backwards compatibility
    if (mode.bits & 0xFFFFFF00) != 0 {
        try!(from_nix_result(unsafe { ioctl::set_mode_u32(fd, &mode.bits) }));
    } else {
        let bits: u8 = mode.bits as u8;
        try!(from_nix_result(unsafe { ioctl::set_mode_u8(fd, &bits) }));
    }
    Ok(())
}

pub fn get_lsb_first(fd: RawFd) -> io::Result<u8> {
    let mut lsb_first: u8 = 0;
    try!(from_nix_result(unsafe { ioctl::get_lsb_first(fd, &mut lsb_first) }));
    Ok(lsb_first)
}

pub fn set_lsb_first(fd: RawFd, lsb_first: bool) -> io::Result<()> {
    let lsb_first_value: u8 = if lsb_first {
        1
    } else {
        0
    };
    try!(from_nix_result(unsafe { ioctl::set_lsb_first(fd, &lsb_first_value) }));
    Ok(())
}

pub fn get_bits_per_word(fd: RawFd) -> io::Result<u8> {
    let mut bits_per_word: u8 = 0;
    try!(from_nix_result(unsafe { ioctl::get_bits_per_word(fd, &mut bits_per_word) }));
    Ok(bits_per_word)
}

pub fn set_bits_per_word(fd: RawFd, bits_per_word: u8) -> io::Result<()> {
    try!(from_nix_result(unsafe { ioctl::set_bits_per_word(fd, &bits_per_word) }));
    Ok(())
}

pub fn get_max_speed_hz(fd: RawFd) -> io::Result<u32> {
    let mut max_speed_hz: u32 = 0;
    try!(from_nix_result(unsafe { ioctl::get_max_speed_hz(fd, &mut max_speed_hz) }));
    Ok(max_speed_hz)
}

pub fn set_max_speed_hz(fd: RawFd, max_speed_hz: u32) -> io::Result<()> {
    try!(from_nix_result(unsafe { ioctl::set_max_speed_hz(fd, &max_speed_hz) }));
    Ok(())
}

pub fn transfer(fd: RawFd, transfer: &mut SpidevTransfer) -> io::Result<()> {
    let mut raw_transfer = transfer.as_spi_ioc_transfer();

    // The kernel will directly modify the rx_buf of the SpidevTransfer
    // rx_buf if present, so there is no need to do any additional work
    try!(from_nix_result(unsafe { ioctl::spidev_transfer(fd, &mut raw_transfer) }));
    Ok(())
}

pub fn transfer_multiple(fd: RawFd, transfers: &Vec<SpidevTransfer>) -> io::Result<()> {
    // create a boxed slice containing several spi_ioc_transfers
    let mut raw_transfers = transfers.iter()
                                     .map(|transfer| transfer.as_spi_ioc_transfer())
                                     .collect::<Vec<_>>()
                                     .into_boxed_slice();
    let tot_size = raw_transfers.len() * mem::size_of::<spi_ioc_transfer>();

    try!(from_nix_result(unsafe {
        ioctl::spidev_transfer_buf(fd, raw_transfers.as_mut_ptr(), tot_size)
    }));
    Ok(())
}
