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
    info!("Dual Mono I2S Start");

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

    let (sai_a_sub, sai_b_sub) = sai::split_subblocks(p.SAI1);

    // Стандартный конфиг I2S (Stereo 32-bit)
    let mut cfg = sai::Config::default();
    cfg.bit_order = BitOrder::MsbFirst;
    cfg.slot_count = word::U4(2);         // ЦАП ожидает 2 слота
    cfg.data_size = DataSize::Data32;
    cfg.frame_length = 64;                // 2 слота * 32 бита
    cfg.frame_sync_active_level_length = word::U7(32);
    cfg.frame_sync_offset = FrameSyncOffset::BeforeFirstBit; // Стандарт I2S
    cfg.frame_sync_polarity = FrameSyncPolarity::ActiveLow;  // Стандарт I2S
    cfg.clock_strobe = ClockStrobe::Falling; // Данные меняются по спаду, сэмплируются по фронту
    cfg.master_clock_divider = MasterClockDivider::DIV1;

    // MASTER - Блок A (Левый ЦАП)
    let mut sai_a = Sai::new_asynchronous_with_mclk(
        sai_a_sub,
        p.PE5, // SCK
        p.PE6, // SD_A
        p.PE4, // FS
        p.PE2, // MCLK
        p.GPDMA1_CH1,
        &mut buf_a,
        Irqs,
        cfg,
    );

    // SLAVE - Блок B (Правый ЦАП)
    let mut cfg_b = cfg;
    cfg_b.sync_input = SyncInput::Internal;

    let mut sai_b = Sai::new_synchronous(
        sai_b_sub,
        p.PE3, // SD_B
        p.GPDMA1_CH2,
        &mut buf_b,
        Irqs,
        cfg_b,
    );

    // Подготовка тестового сигнала 1 кГц
    let sample_rate = 48_000;
    let freq = 1_000;
    let period_samples = sample_rate / freq;
    let half_period = period_samples / 2;

    let mut data_a = [0u32; 128];
    let mut data_b = [0u32; 128];

    for i in (0..128).step_by(2) {
        let v = if (i % period_samples) < half_period {
            0x40000000 // Не наглеем с амплитудой для теста
        } else {
            0xC0000000 // Отрицательное значение в i32
        };

        // ЦАП №1 (Блок А): Звук в Левом канале, Тишина в Правом
        data_a[i] = v;
        data_a[i + 1] = 0;

        // ЦАП №2 (Блок Б): Тишина в Левом канале, Звук в Правом
        data_b[i] = 0;
        data_b[i + 1] = v;
    }

    info!("Starting Parallel Playback (Dual Mono)");

    loop {
        // Используем join для одновременной передачи в оба DMA канала
        let _ = join(
            sai_a.write(&data_a),
                     sai_b.write(&data_b),
        ).await;
    }
}
