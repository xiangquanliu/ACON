// Copyright (C) 2023 Intel Corporation
// SPDX-License-Identifier: Apache-2.0
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate scopeguard;

use crate::config::Config;
use anyhow::{anyhow, Result};
use nix::{
    sys::reboot::{self, RebootMode},
    unistd::{self, ForkResult, Gid, Pid, Uid},
};
use std::{fs, os::unix::net::UnixStream, path::Path};
use tokio::runtime::Builder;

mod config;
mod container;
mod image;
mod io;
mod ipc;
mod mount;
mod pod;
#[cfg(feature = "interactive")]
mod pty;
mod report;
mod restful;
mod server;
mod utils;

fn start_service() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::new();
    config.parse_cmdline(None)?;

    let gid = Gid::from_raw(1);
    let uid = Uid::from_raw(1);
    let (pstream, cstream) = UnixStream::pair()?;

    match unsafe { unistd::fork() } {
        Ok(ForkResult::Parent { child: _ }) => {
            let path = Path::new("/run/user").join(format!("{}", uid));
            fs::create_dir_all(&path)?;
            unistd::chown(&path, Some(uid), Some(gid))?;

            pstream.set_nonblocking(true)?;
            let rt = Builder::new_current_thread().enable_all().build()?;
            rt.block_on(server::start_server(pstream, &config))?;
            rt.shutdown_background();

            Ok(())
        }
        Ok(ForkResult::Child) => {
            cstream.set_nonblocking(true)?;
            unistd::setresgid(gid, gid, gid)?;
            unistd::setresuid(uid, uid, uid)?;
            prctl::set_name("rpc_server").map_err(|e| anyhow!(e.to_string()))?;

            let rt = Builder::new_current_thread().enable_all().build()?;
            rt.block_on(restful::run_server(cstream, config.tcp_port))?;

            Ok(())
        }
        Err(errno) => {
            eprintln!("Start service error, errno = {errno}.");
            Err("Start service error, errno = {errno}.".into())
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Uncomment it to debug.
    // env_logger::init_from_env(env_logger::Env::default().default_filter_or("debug"));
    mount::mount_rootfs()?;

    start_service()?;

    if unistd::getpid() == Pid::from_raw(1) {
        reboot::reboot(RebootMode::RB_POWER_OFF)?;
    }
    Ok(())
}
