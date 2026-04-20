#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::time::Hertz;
use embassy_stm32::{Config, bind_interrupts, dma, peripherals, sai};
use embassy_stm32::sai::{
    BitOrder, ClockStrobe, DataSize, FrameSyncOffset, FrameSyncPolarity,
    MasterClockDivider, Sai, SyncInput, word,
};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct IrqsA {
    GPDMA1_CHANNEL1 => dma::InterruptHandler<peripherals::GPDMA1_CH1>;
});

bind_interrupts!(struct IrqsB {
    GPDMA1_CHANNEL2 => dma::InterruptHandler<peripherals::GPDMA1_CH2>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Dual SAI codec-style start");

    // ---------------- CLOCK ----------------
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
            divp: Some(PllDiv::DIV25),
                               divq: None,
                               divr: None,
        });

        config.rcc.mux.sai1sel = mux::Saisel::PLL2_P;
    }

    let p = embassy_stm32::init(config);

    // ---------------- BUFFERS ----------------
    let mut buf_a = [0u32; 256];
    let mut buf_b = [0u32; 256];

    // ---------------- SPLIT SAI ----------------
    let (sai_a, sai_b) = sai::split_subblocks(p.SAI1);

    // ---------------- COMMON CONFIG ----------------
    let mut cfg = sai::Config::default();

    cfg.bit_order = BitOrder::MsbFirst;
    cfg.data_size = DataSize::Data32;

    // 🔥 MONO MODE (IMPORTANT)
    cfg.slot_count = word::U4(1);
    cfg.frame_length = 32;

    cfg.frame_sync_offset = FrameSyncOffset::BeforeFirstBit;
    cfg.frame_sync_polarity = FrameSyncPolarity::ActiveLow;

    cfg.clock_strobe = ClockStrobe::Rising;
    cfg.master_clock_divider = MasterClockDivider::DIV1;

    // ---------------- MASTER (LEFT DAC) ----------------
    let mut sai_a = Sai::new_asynchronous_with_mclk(
        sai_a,
        p.PE5, // SCK
        p.PE6, // SD_A → DAC1
        p.PE4, // FS
        p.PE2, // MCLK
        p.GPDMA1_CH1,
        &mut buf_a,
        IrqsA,
        cfg,
    );

    // ---------------- SLAVE (RIGHT DAC) ----------------
    let mut cfg_b = cfg.clone();
    cfg_b.sync_input = SyncInput::Internal;

    let mut sai_b = Sai::new_synchronous(
        sai_b,
        p.PE3, // SD_B → DAC2
        p.GPDMA1_CH2,
        &mut buf_b,
        IrqsB,
        cfg_b,
    );

    // ---------------- TEST SIGNAL ----------------
    let sample_rate = 48_000;
    let freq = 1000;
    let period = sample_rate / freq;

    let mut left = [0u32; 128];
    let mut right = [0u32; 128];

    for i in 0..128 {
        let v = if (i % period) < (period / 2) {
            0x7FFFFFFF
        } else {
            0x80000000
        };

        left[i] = v;
        right[i] = v;
    }

    info!("Priming slave (B)");

    // 🔥 CRITICAL: prime B first
    let _ = sai_b.write(&right).await;

    info!("Starting master (A)");

    let _ = sai_a.write(&left).await;

    info!("Running sync audio");

    loop {
        // continuous aligned streaming
        let _ = sai_a.write(&left).await;
        let _ = sai_b.write(&right).await;
    }
}
