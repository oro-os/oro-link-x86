use crate::uc;
use defmt::{debug, info};
use embassy_executor::Spawner;
use embassy_stm32::{
	bind_interrupts,
	dma::NoDma,
	gpio::{Input, Level, Output, OutputOpenDrain, Pull, Speed},
	i2c::{self, I2c},
	peripherals, rng,
	rtc::{self, RtcClockSource},
	spi::{self, Spi},
	time::Hertz,
	usart, Config,
};
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;

bind_interrupts!(struct Irqs {
	I2C1_EV => i2c::InterruptHandler<peripherals::I2C1>;
	USART3 => usart::InterruptHandler<peripherals::USART3>;
	HASH_RNG => rng::InterruptHandler<peripherals::RNG>;
});

pub async fn init(
	_spawner: &Spawner,
) -> (
	impl uc::DebugLed,
	impl uc::SystemUnderTest,
	impl uc::Monitor,
	impl uc::EthernetDriver,
	impl uc::SystemEthernet,
	impl uc::WallClock,
	impl uc::Rng,
	impl uc::UartTx,
	impl uc::UartRx,
) {
	let mut config = Config::default();
	config.rcc.rtc = Some(RtcClockSource::LSI);
	config.rcc.hse = Some(Hertz::mhz(26));
	config.rcc.bypass_hse = false;
	config.rcc.hclk = Some(Hertz(168409091));
	config.rcc.sys_ck = Some(Hertz(168409091));
	config.rcc.pll48 = true;
	config.rcc.lsi = true;

	let p = embassy_stm32::init(config);

	info!("initializing STM32f479vg...");

	let mut ind_on = Output::new(p.PB4, Level::Low, Speed::Low);
	ind_on.set_high();
	::core::mem::forget(ind_on); // Keep it high even after we return.

	let i2c = I2c::new(
		p.I2C1,
		p.PB6,
		p.PB7,
		Irqs,
		NoDma,
		NoDma,
		Hertz(400_000),
		Default::default(),
	);

	let indicators = crate::chip::is31fl3218::Is31fl3218::new(i2c);
	let indicators =
		crate::chip::is31fl3218::IndicatorLights::<_, 0, 1, 17, 12, 13, 11, 16, 14, 15>::new(
			indicators,
		);

	info!("... indicators INIT");

	let wall_clock = rtc::Rtc::new(p.RTC, rtc::RtcConfig::default());
	info!("... rtc INIT");

	// Let OLED power on (affects first power-on cycle, typically)
	Timer::after(Duration::from_millis(50)).await;

	let mut oled_en = Output::new(p.PE2, Level::Low, Speed::Low);
	let mut oled_rst = Output::new(p.PC13, Level::Low, Speed::Low);
	oled_en.set_high();
	oled_rst.set_high();
	// Keep it high even after we return
	::core::mem::forget(oled_en);
	::core::mem::forget(oled_rst);

	let mut oledconf = spi::Config::default();
	oledconf.mode = spi::MODE_0;
	oledconf.bit_order = spi::BitOrder::MsbFirst;
	oledconf.frequency = Hertz(8_000_000);

	let mut oled = crate::chip::ssd1362::SSD1362::new(
		ExclusiveDevice::new(
			Spi::new_txonly(p.SPI2, p.PD3, p.PC3, NoDma, NoDma, oledconf),
			OutputOpenDrain::new(p.PB9, Level::High, Speed::VeryHigh, Pull::None),
			Delay,
		),
		Output::new(p.PC14, Level::High, Speed::VeryHigh),
		true, // do a flip
		137,  // gamma value
	)
	.unwrap();

	oled.on().unwrap();
	oled.clear().unwrap();

	info!("... oled INIT");

	let monitor =
		crate::uc::helper::monitor::three_indicators_oled_256x64::ThreeIndicatorsOled256x64::new(
			oled, indicators,
		);

	info!("... monitor INIT");

	let mut exteth_en = Output::new(p.PD7, Level::Low, Speed::Low);
	let mut exteth_xfrm_en = Output::new(p.PD2, Level::Low, Speed::Low);
	exteth_en.set_high();
	exteth_xfrm_en.set_high();
	// Keep them high even after we return.
	::core::mem::forget(exteth_en);
	::core::mem::forget(exteth_xfrm_en);

	info!("... external ethernet transformer INIT");

	let mut extconf = spi::Config::default();
	extconf.mode = spi::MODE_0;
	extconf.bit_order = spi::BitOrder::MsbFirst;
	extconf.frequency = Hertz(8_000_000);

	// TODO use DMA
	let extspi = Spi::new(p.SPI3, p.PC10, p.PC12, p.PC11, NoDma, NoDma, extconf);

	info!("... external ethernet comms INIT");

	let extdev = ExclusiveDevice::new(
		extspi,
		OutputOpenDrain::new(p.PA15, Level::High, Speed::VeryHigh, Pull::None),
		Delay,
	);

	info!("... external ethernet dev INIT");

	let extmac = super::get_exteth_mac();

	debug!("... external MAC: {:?}", extmac);

	let exteth = crate::chip::enc28j60::Enc28j60::new(
		extdev,
		Some(Output::new(p.PD0, Level::High, Speed::VeryHigh)),
		extmac,
	);

	info!("... external ethernet INIT");

	let mut syseth_en = Output::new(p.PA2, Level::Low, Speed::Low);
	let mut syseth_xfrm_en = Output::new(p.PA3, Level::Low, Speed::Low);
	syseth_en.set_high();
	syseth_xfrm_en.set_high();
	// Keep them high even after we return.
	::core::mem::forget(syseth_en);
	::core::mem::forget(syseth_xfrm_en);

	info!("... system ethernet transformer INIT");

	let mut sysconf = spi::Config::default();
	sysconf.mode = spi::MODE_0;
	sysconf.bit_order = spi::BitOrder::MsbFirst;
	sysconf.frequency = Hertz(8_000_000);

	// TODO use DMA
	let sysspi = Spi::new(p.SPI1, p.PA5, p.PA7, p.PA6, NoDma, NoDma, sysconf);

	info!("... system ethernet comms INIT");

	let sysdev = ExclusiveDevice::new(
		sysspi,
		OutputOpenDrain::new(p.PA4, Level::High, Speed::VeryHigh, Pull::None),
		Delay,
	);

	info!("... system ethernet dev INIT");

	let syseth = crate::chip::enc28j60::Enc28j60::new(
		sysdev,
		Some(Output::new(p.PB1, Level::High, Speed::VeryHigh)),
		[b'.', b'o', b'O', b'D', b'E', b'V'],
	);

	info!("... system ethernet INIT");

	let system = super::SystemUnderTest::new(
		Output::new(p.PC9, Level::Low, Speed::Low),
		Output::new(p.PC8, Level::Low, Speed::Low),
		Output::new(p.PD6, Level::Low, Speed::Low),
		Output::new(p.PD4, Level::Low, Speed::Low),
		Input::new(p.PD5, Pull::Up),
	);

	info!("... system under test INIT");

	let debug_led = super::DebugLed::new(Output::new(p.PE12, Level::Low, Speed::Low));

	info!("... debug led INIT");

	let rng_gen = rng::Rng::new(p.RNG, Irqs);

	info!("... rng INIT");

	let mut syscom_config = usart::Config::default();
	syscom_config.baudrate = 38400;
	syscom_config.data_bits = usart::DataBits::DataBits8;
	syscom_config.stop_bits = usart::StopBits::STOP1;
	syscom_config.parity = usart::Parity::ParityNone;

	// TODO maybe expose this somehow so that the test runner can turn it on and off.
	let mut syscom_on = Output::new(p.PD8, Level::Low, Speed::Low);
	syscom_on.set_high();
	::core::mem::forget(syscom_on); // Keep it high even after we return.

	let (syscom_tx, syscom_rx) = usart::Uart::new_with_rtscts(
		p.USART3,
		p.PB11,
		p.PB10,
		Irqs,
		p.PB14,
		p.PB13,
		p.DMA1_CH3,
		p.DMA1_CH1,
		syscom_config,
	)
	.expect("failed to create SUT usart pair")
	.split();

	const DMA_BUF_SIZE: usize = 256;
	static mut DMA_BUF: [u8; DMA_BUF_SIZE] = [0; DMA_BUF_SIZE];
	let syscom_rx = syscom_rx.into_ring_buffered(unsafe { DMA_BUF.as_mut() });

	info!("... system com INIT");

	(
		debug_led, system, monitor, exteth, syseth, wall_clock, rng_gen, syscom_tx, syscom_rx,
	)
}
