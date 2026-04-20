#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, dma, peripherals, sai, Config};
use embassy_stm32::sai::{
    BitOrder, ClockStrobe, DataSize, FrameSyncOffset, FrameSyncPolarity,
    MasterClockDivider, Sai, SyncInput, word,
};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    GPDMA1_CHANNEL1 => dma::InterruptHandler<peripherals::GPDMA1_CH1>;
    GPDMA1_CHANNEL2 => dma::InterruptHandler<peripherals::GPDMA1_CH2>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("SAI dual (join) start");

    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;

        config.rcc.hse = Some(Hse {
            freq: Hertz(8_000_000),
                              mode: HseMode::Oscillator,
        });

        config.rcc.pll2 = Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV5,
            mul: PllMul::MUL192,
            divp: Some(PllDiv::DIV25), // 12.288 MHz
                               divq: None,
                               divr: None,
        });

        config.rcc.mux.sai1sel = mux::Saisel::PLL2_P;
    }

    let p = embassy_stm32::init(config);

    let mut buf_a = [0u32; 256];
    let mut buf_b = [0u32; 256];

    let (sai_a, sai_b) = sai::split_subblocks(p.SAI1);

    let mut cfg = sai::Config::default();
    cfg.bit_order = BitOrder::MsbFirst;
    cfg.slot_count = word::U4(2);
    cfg.data_size = DataSize::Data32;
    cfg.frame_length = 64;
    cfg.frame_sync_active_level_length = word::U7(32);
    cfg.master_clock_divider = MasterClockDivider::DIV1;
    cfg.clock_strobe = ClockStrobe::Rising;
    cfg.frame_sync_offset = FrameSyncOffset::BeforeFirstBit;
    cfg.frame_sync_polarity = FrameSyncPolarity::ActiveHigh;

    // MASTER
    let mut sai_a = Sai::new_asynchronous_with_mclk(
        sai_a,
        p.PE5,
        p.PE6,
        p.PE4,
        p.PE2,
        p.GPDMA1_CH1,
        &mut buf_a,
        Irqs,
        cfg,
    );

    // SLAVE
    let mut cfg_b = cfg;
    cfg_b.sync_input = SyncInput::Internal;

    let mut sai_b = Sai::new_synchronous(
        sai_b,
        p.PE3,
        p.GPDMA1_CH2,
        &mut buf_b,
        Irqs,
        cfg_b,
    );

    // --- тест сигнал ---
    let mut data_a = [0u32; 128];
    let mut data_b = [0u32; 128];

    for i in (0..128).step_by(2) {
        data_a[i] = 0x7FFFFFFF;
        data_a[i + 1] = 0;

        data_b[i] = 0x80000000;
        data_b[i + 1] = 0;
    }

    // первый запуск
    join(
        sai_a.write(&data_a),
         sai_b.write(&data_b),
    ).await;

    loop {
        join(
            sai_a.write(&data_a),
             sai_b.write(&data_b),
        ).await;
    }
}
