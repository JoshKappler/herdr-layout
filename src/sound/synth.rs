//! Synthesizes notification audio as 16-bit mono WAV bytes: a short siren
//! for requests, a soft ding for completions, and an optional morse-style
//! position suffix (long beeps count the sidebar space top-down, short
//! beeps count the tab inside that space).

use super::{SidebarPosition, Sound};

const SAMPLE_RATE: u32 = 44_100;

// Fluent-operator morse pacing (~22 WPM): a dit is 55 ms, a dah three dits,
// one dit of silence between beeps, three dits between the space group and
// the tab group.
const DIT_MS: u32 = 55;
const DAH_MS: u32 = 3 * DIT_MS;
const ELEMENT_GAP_MS: u32 = DIT_MS;
const GROUP_GAP_MS: u32 = 3 * DIT_MS;
const BASE_TO_MORSE_GAP_MS: u32 = 250;
const STANDALONE_MORSE_LEAD_IN_MS: u32 = 150;
const MORSE_HZ: f32 = 700.0;
const MORSE_AMP: f32 = 0.38;
const TONE_EDGE_MS: u32 = 4;

const DING_MS: u32 = 380;
const SIREN_SWEEP_MS: u32 = 220;
const SIREN_SWEEPS: u32 = 2;

/// The built-in notification sound, with the morse position suffix when the
/// triggering pane's sidebar location is known.
pub fn render_notification_wav(sound: Sound, position: Option<SidebarPosition>) -> Vec<u8> {
    let mut samples = Vec::new();
    match sound {
        Sound::Done => push_ding(&mut samples),
        Sound::Request => push_siren(&mut samples),
    }
    if let Some(position) = position {
        push_silence(&mut samples, BASE_TO_MORSE_GAP_MS);
        push_morse(&mut samples, position);
    }
    wav_from_samples(&samples)
}

/// The morse position suffix alone, played after a user-configured sound
/// file that the audio player renders separately.
pub fn render_morse_wav(position: SidebarPosition) -> Vec<u8> {
    let mut samples = Vec::new();
    push_silence(&mut samples, STANDALONE_MORSE_LEAD_IN_MS);
    push_morse(&mut samples, position);
    wav_from_samples(&samples)
}

fn push_morse(samples: &mut Vec<f32>, position: SidebarPosition) {
    for i in 0..position.space {
        if i > 0 {
            push_silence(samples, ELEMENT_GAP_MS);
        }
        push_beep(samples, MORSE_HZ, DAH_MS, MORSE_AMP);
    }
    push_silence(samples, GROUP_GAP_MS);
    for i in 0..position.tab {
        if i > 0 {
            push_silence(samples, ELEMENT_GAP_MS);
        }
        push_beep(samples, MORSE_HZ, DIT_MS, MORSE_AMP);
    }
}

/// Soft bell strike: a decaying fundamental plus a faint, faster-dying
/// second partial.
fn push_ding(samples: &mut Vec<f32>) {
    let total = ms_samples(DING_MS);
    let edge = ms_samples(TONE_EDGE_MS);
    for i in 0..total {
        let t = i as f32 / SAMPLE_RATE as f32;
        let fundamental = 0.32 * (-t / 0.110).exp() * (std::f32::consts::TAU * 1318.51 * t).sin();
        let partial = 0.10 * (-t / 0.055).exp() * (std::f32::consts::TAU * 2637.02 * t).sin();
        samples.push(edge_envelope(i, total, edge) * (fundamental + partial));
    }
}

/// Short alarm: the pitch glides 650 to 950 Hz and back with continuous
/// phase, twice.
fn push_siren(samples: &mut Vec<f32>) {
    const LOW_HZ: f32 = 650.0;
    const HIGH_HZ: f32 = 950.0;
    let total = ms_samples(SIREN_SWEEP_MS * SIREN_SWEEPS);
    let sweep_len = ms_samples(SIREN_SWEEP_MS);
    let edge = ms_samples(2 * TONE_EDGE_MS);
    let mut phase = 0.0f32;
    for i in 0..total {
        let sweep_pos = (i % sweep_len) as f32 / sweep_len as f32;
        let rise_fall = 1.0 - (2.0 * sweep_pos - 1.0).abs();
        let freq = LOW_HZ + (HIGH_HZ - LOW_HZ) * rise_fall;
        phase += std::f32::consts::TAU * freq / SAMPLE_RATE as f32;
        samples.push(0.5 * edge_envelope(i, total, edge) * phase.sin());
    }
}

fn push_beep(samples: &mut Vec<f32>, freq: f32, ms: u32, amp: f32) {
    let total = ms_samples(ms);
    let edge = ms_samples(TONE_EDGE_MS).min(total / 2);
    for i in 0..total {
        let t = i as f32 / SAMPLE_RATE as f32;
        let tone = (std::f32::consts::TAU * freq * t).sin();
        samples.push(amp * edge_envelope(i, total, edge) * tone);
    }
}

fn push_silence(samples: &mut Vec<f32>, ms: u32) {
    samples.resize(samples.len() + ms_samples(ms), 0.0);
}

/// Raised-cosine attack and release so tones never click on and off.
fn edge_envelope(i: usize, total: usize, edge: usize) -> f32 {
    if edge == 0 {
        return 1.0;
    }
    let ramp = |n: usize| 0.5 - 0.5 * (std::f32::consts::PI * n as f32 / edge as f32).cos();
    if i < edge {
        ramp(i)
    } else if i >= total.saturating_sub(edge) {
        ramp(total.saturating_sub(1).saturating_sub(i))
    } else {
        1.0
    }
}

fn ms_samples(ms: u32) -> usize {
    (u64::from(SAMPLE_RATE) * u64::from(ms) / 1000) as usize
}

fn wav_from_samples(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let quantized = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        wav.extend_from_slice(&quantized.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_sample_count(wav: &[u8]) -> usize {
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(wav.len(), 44 + data_len);
        data_len / 2
    }

    #[test]
    fn wav_header_is_valid_pcm16_mono() {
        let wav = render_notification_wav(Sound::Done, None);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes(wav[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(wav[24..28].try_into().unwrap()),
            SAMPLE_RATE
        );
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
        assert_eq!(&wav[36..40], b"data");
    }

    #[test]
    fn done_wav_lasts_the_ding_duration() {
        let wav = render_notification_wav(Sound::Done, None);
        assert_eq!(wav_sample_count(&wav), ms_samples(DING_MS));
    }

    #[test]
    fn request_wav_lasts_the_siren_duration() {
        let wav = render_notification_wav(Sound::Request, None);
        assert_eq!(
            wav_sample_count(&wav),
            ms_samples(SIREN_SWEEP_MS * SIREN_SWEEPS)
        );
    }

    #[test]
    fn morse_suffix_counts_spaces_as_dahs_and_tabs_as_dits() {
        let position = SidebarPosition { space: 3, tab: 2 };
        let wav = render_notification_wav(Sound::Done, Some(position));
        let expected = ms_samples(DING_MS)
            + ms_samples(BASE_TO_MORSE_GAP_MS)
            + 3 * ms_samples(DAH_MS) // three dahs for the third space down
            + 2 * ms_samples(ELEMENT_GAP_MS)
            + ms_samples(GROUP_GAP_MS)
            + 2 * ms_samples(DIT_MS) // two dits for the second tab
            + ms_samples(ELEMENT_GAP_MS);
        assert_eq!(wav_sample_count(&wav), expected);
    }

    #[test]
    fn standalone_morse_wav_covers_position_one_one() {
        let wav = render_morse_wav(SidebarPosition { space: 1, tab: 1 });
        let expected = ms_samples(STANDALONE_MORSE_LEAD_IN_MS)
            + ms_samples(DAH_MS)
            + ms_samples(GROUP_GAP_MS)
            + ms_samples(DIT_MS);
        assert_eq!(wav_sample_count(&wav), expected);
    }

    #[test]
    fn samples_stay_inside_pcm_range() {
        for wav in [
            render_notification_wav(Sound::Done, Some(SidebarPosition { space: 5, tab: 4 })),
            render_notification_wav(Sound::Request, Some(SidebarPosition { space: 1, tab: 8 })),
        ] {
            for chunk in wav[44..].chunks_exact(2) {
                let sample = i16::from_le_bytes(chunk.try_into().unwrap());
                assert!(sample > i16::MIN, "sample clipped past full scale");
            }
        }
    }
}
