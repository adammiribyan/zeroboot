/// Minimal 16550 UART emulation for guest serial I/O.
/// We only need enough to capture guest output and send input.
use std::collections::VecDeque;

const IER: u16 = 1;
const IIR: u16 = 2;
const LCR: u16 = 3;
const MCR: u16 = 4;
const LSR: u16 = 5;
const MSR: u16 = 6;

const LSR_DATA_READY: u8 = 0x01;
const LSR_THR_EMPTY: u8 = 0x20;
const LSR_IDLE: u8 = 0x40;

pub struct Serial {
    pub output: Vec<u8>,
    input: VecDeque<u8>,
    ier: u8,
    lcr: u8,
    mcr: u8,
    divisor_latch_low: u8,
    divisor_latch_high: u8,
    scratch: u8,
}

impl Serial {
    pub fn new() -> Self {
        Self {
            output: Vec::new(),
            input: VecDeque::new(),
            ier: 0,
            lcr: 0,
            mcr: 0,
            divisor_latch_low: 0,
            divisor_latch_high: 0,
            scratch: 0,
        }
    }

    pub fn queue_input(&mut self, data: &[u8]) {
        self.input.extend(data);
    }

    pub fn has_pending_input(&self) -> bool {
        !self.input.is_empty()
    }

    pub fn input_len(&self) -> usize {
        self.input.len()
    }

    pub fn thri_enabled(&self) -> bool {
        self.ier & 0x02 != 0 // ETBEI: Enable Transmitter Holding Register Empty Interrupt
    }

    pub fn set_ier_data_ready(&mut self, enable: bool) {
        if enable {
            self.ier |= 0x01; // Enable received data available interrupt
        } else {
            self.ier &= !0x01;
        }
    }

    fn dlab(&self) -> bool {
        self.lcr & 0x80 != 0
    }

    pub fn read(&mut self, offset: u16) -> u8 {
        match offset {
            0 => {
                if self.dlab() {
                    self.divisor_latch_low
                } else {
                    self.input.pop_front().unwrap_or(0)
                }
            }
            IER => {
                if self.dlab() {
                    self.divisor_latch_high
                } else {
                    self.ier
                }
            }
            IIR => {
                // Check if there's data ready and IER has data-available bit
                if !self.input.is_empty() && (self.ier & 0x01) != 0 {
                    0x04 // Received data available interrupt
                } else if (self.ier & 0x02) != 0 {
                    0x02 // THR empty interrupt
                } else {
                    0x01 // No interrupt pending
                }
            }
            LCR => self.lcr,
            MCR => self.mcr,
            LSR => {
                let mut lsr = LSR_THR_EMPTY | LSR_IDLE;
                if !self.input.is_empty() {
                    lsr |= LSR_DATA_READY;
                }
                lsr
            }
            MSR => 0,
            7 => self.scratch,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u16, value: u8) {
        match offset {
            0 => {
                if self.dlab() {
                    self.divisor_latch_low = value;
                } else {
                    self.output.push(value);
                }
            }
            IER => {
                if self.dlab() {
                    self.divisor_latch_high = value;
                } else {
                    self.ier = value;
                }
            }
            LCR => self.lcr = value,
            MCR => self.mcr = value,
            7 => self.scratch = value,
            _ => {}
        }
    }
}
