#![feature(never_type, async_fn_in_trait)]

use async_std::{
	io,
	net::{TcpListener, TcpStream},
	prelude::*,
	task,
};
use envconfig::Envconfig;
use link_protocol::{
	channel::{negotiate, RWError, Side as ChannelSide},
	Error as ProtoError, Packet,
};
use log::{debug, error, info, warn};
use rand::{rngs::OsRng, RngCore};

#[derive(Envconfig)]
struct Config {
	#[envconfig(from = "LINK_SERVER_PORT", default = "1337")]
	pub link_server_port: u16,
	#[envconfig(from = "LINK_SERVER_BIND", default = "0.0.0.0")]
	pub link_server_bind: String,
	#[cfg(target_os = "linux")]
	#[envconfig(from = "USE_JOURNALD", default = "0")]
	pub use_journald: u8,
}

async fn task_process_oro_link(stream: TcpStream) -> Result<(), ProtoError<io::Error>> {
	debug!("incoming oro link connection");

	let receiver = io::BufReader::new(stream.clone());
	let sender = io::BufWriter::new(stream);

	debug!("created streams; negotiating");

	let (mut sender, mut receiver) =
		match negotiate(sender, receiver, &mut OsRng, ChannelSide::Server).await {
			Ok(v) => v,
			Err(RWError::Read(err) | RWError::Write(err)) => {
				error!("failed to negotiate encrypted channel with link: {:?}", err);
				return Err(err);
			}
		};

	debug!("negotiated; beginning communications");

	loop {
		let packet = receiver.receive().await?;

		match packet {
			Packet::LinkOnline { uid, version } => {
				info!("oro link came online");
				info!("    link firmware version: {}", version);
				info!("    link UID:              {}", ::hex::encode_upper(uid));

				// XXX DEBUG changing scene
				sender
					.send(Packet::SetScene(link_protocol::Scene::Logo))
					.await?;
				async_std::task::sleep(core::time::Duration::from_secs(3)).await;
				sender
					.send(Packet::SetScene(link_protocol::Scene::Log))
					.await?;
				sender
					.send(Packet::Log(link_protocol::LogEntry::Info(
						"booting machine...".into(),
					)))
					.await?;
				sender
					.send(Packet::SetPowerState(link_protocol::PowerState::On))
					.await?;
				sender.send(Packet::PressPower).await?;
				async_std::task::sleep(core::time::Duration::from_secs(5)).await;
				sender
					.send(Packet::StartTestSession {
						total_tests: 1337,
						author: "Josh Junon".into(),
						title: "test: daemon protocol".into(),
						ref_id: "abcd1234abcd1234abcd1234abcd1234".into(),
					})
					.await?;
				sender
					.send(Packet::SetScene(link_protocol::Scene::Test))
					.await?;

				for (i, name) in [
					"test_protocol_proc_macro",
					"test_test_harness",
					"test_hid_mouse",
					"test_hid_keyboard",
				]
				.into_iter()
				.cycle()
				.take(1337)
				.enumerate()
				{
					if (i % 50) == 0 && i > 0 {
						sender.send(Packet::PressReset).await?;
					}
					sender.send(Packet::StartTest { name: name.into() }).await?;
					async_std::task::sleep(core::time::Duration::from_millis(
						OsRng.next_u64() % 300,
					))
					.await;
				}

				sender.send(Packet::ResetLink).await?;
			}
			unknown => warn!("dropping unknown packet: {:?}", unknown),
		}
	}
}

async fn task_accept_oro_link_tcp(bind_host: String, port: u16) -> Result<(), io::Error> {
	let listener = TcpListener::bind((bind_host.as_str(), port)).await?;
	let mut incoming = listener.incoming();

	info!("listening for link connections on {}:{}", bind_host, port);

	while let Some(stream) = incoming.next().await {
		let stream = stream?;
		task::spawn(async move {
			if let Err(err) = task_process_oro_link(stream).await {
				error!("oro link peer connection encountered error: {:?}", err);
			}
		});
	}

	Ok(())
}

#[async_std::main]
async fn main() -> Result<!, io::Error> {
	let config = Config::init_from_env().unwrap();

	log::set_max_level(log::LevelFilter::Trace);

	#[allow(unused)]
	let should_fallback = true;

	#[cfg(target_os = "linux")] // FIXME: This isn't working.
	let should_fallback = {
		if config.use_journald != 0 {
			systemd_journal_logger::JournalLog::default()
				.with_extra_fields(vec![("VERSION", env!("CARGO_PKG_VERSION"))])
				.with_syslog_identifier("oro-linkd".to_string())
				.install()
				.expect("failed to start journald logger");

			false
		} else {
			true
		}
	};

	if should_fallback {
		stderrlog::new()
			//.module(module_path!())
			.show_module_names(true)
			.verbosity(log::max_level())
			.timestamp(stderrlog::Timestamp::Millisecond)
			.init()
			.expect("failed to start stderr logger");
	}

	info!("starting oro-linkd version {}", env!("CARGO_PKG_VERSION"));

	task::spawn(async move {
		if let Err(err) =
			task_accept_oro_link_tcp(config.link_server_bind, config.link_server_port).await
		{
			error!("oro link tcp server error: {:?}", err);
		}
		warn!("oro link tcp server has shut down; terminating...");
		std::process::exit(2);
	});

	async_io::Timer::never().await;
	unreachable!();
}
