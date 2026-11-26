use atk_audio::Result;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

type HResult = i32;
type Interface = *mut c_void;

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const DEVICE_ENUMERATOR: Guid = Guid(
    0xbcde0395,
    0xe52f,
    0x467c,
    [0x8e, 0x3d, 0xc4, 0x57, 0x92, 0x91, 0x69, 0x2e],
);
const DEVICE_ENUMERATOR_INTERFACE: Guid = Guid(
    0xa95664d2,
    0x9614,
    0x4f35,
    [0xa7, 0x46, 0xde, 0x8d, 0xb6, 0x36, 0x17, 0xe6],
);
const AUDIO_CLIENT: Guid = Guid(
    0x1cb9ad4c,
    0xdbfa,
    0x4c32,
    [0xb1, 0x78, 0xc2, 0xf5, 0x68, 0xa7, 0x03, 0xb2],
);
const CAPTURE_CLIENT: Guid = Guid(
    0xc8adbd64,
    0xe71e,
    0x48a0,
    [0xa4, 0xde, 0x18, 0x5c, 0x39, 0x5c, 0xd3, 0x17],
);

const IN_PROCESS_SERVER: u32 = 1;
const SHARED_MODE: u32 = 0;
const LOOPBACK: u32 = 0x0002_0000;
const SILENT_PACKET: u32 = 2;
const AUTO_CONVERT_PCM: u32 = 0x8000_0000;
const DEFAULT_SRC_QUALITY: u32 = 0x0800_0000;

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> HResult;
    fn CoUninitialize();
    fn CoCreateInstance(
        class: *const Guid,
        outer: Interface,
        context: u32,
        interface: *const Guid,
        instance: *mut Interface,
    ) -> HResult;
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct Com(Interface);

impl Com {
    // The slot and signature must match the interface returned by Windows.
    unsafe fn method<T: Copy>(&self, slot: usize) -> T {
        let table = unsafe { *self.0.cast::<*const *const c_void>() };
        let function = unsafe { *table.add(slot) };
        unsafe { std::mem::transmute_copy(&function) }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            let release: extern "system" fn(Interface) -> u32 = self.method(2);
            release(self.0);
        }
    }
}

pub struct Recording {
    pub bytes: Vec<u8>,
    pub duration: f64,
    pub rate: u32,
    pub channels: u16,
}

fn check(result: HResult, operation: &str) -> Result<()> {
    if result < 0 {
        return Err(format!(
            "{operation} failed (Windows audio error 0x{:08X})",
            result as u32
        )
        .into());
    }

    Ok(())
}

pub fn record(seconds: f64, endpoint: Option<&str>) -> Result<Recording> {
    unsafe {
        check(CoInitializeEx(null_mut(), 0), "Initializing Windows audio")?;
        let _apartment = Apartment;
        let mut enumerator = null_mut();
        let result = CoCreateInstance(
            &DEVICE_ENUMERATOR,
            null_mut(),
            IN_PROCESS_SERVER,
            &DEVICE_ENUMERATOR_INTERFACE,
            &mut enumerator,
        );
        check(result, "Opening the playback device list")?;
        let enumerator = Com(enumerator);

        let mut device = null_mut();
        let result = if let Some(endpoint) = endpoint {
            let id: Vec<_> = endpoint.encode_utf16().chain([0]).collect();
            let get_device: extern "system" fn(Interface, *const u16, *mut Interface) -> HResult =
                enumerator.method(5);
            get_device(enumerator.0, id.as_ptr(), &mut device)
        } else {
            let default_endpoint: extern "system" fn(
                Interface,
                u32,
                u32,
                *mut Interface,
            ) -> HResult = enumerator.method(4);
            default_endpoint(enumerator.0, 0, 1, &mut device)
        };
        check(result, "Opening the playback device")?;
        let device = Com(device);

        let mut client = null_mut();
        let activate: extern "system" fn(
            Interface,
            *const Guid,
            u32,
            *const c_void,
            *mut Interface,
        ) -> HResult = device.method(3);
        let result = activate(
            device.0,
            &AUDIO_CLIENT,
            IN_PROCESS_SERVER,
            null(),
            &mut client,
        );
        check(result, "Activating the playback device")?;
        record_client(&Com(client), seconds)
    }
}

unsafe fn record_client(client: &Com, seconds: f64) -> Result<Recording> {
    unsafe {
        let channels = 2u16;
        let rate = 48_000u32;
        let bits = 16u16;
        let alignment = channels * (bits / 8);
        let mut format = [0u8; 18];
        format[..2].copy_from_slice(&1u16.to_le_bytes());
        format[2..4].copy_from_slice(&channels.to_le_bytes());
        format[4..8].copy_from_slice(&rate.to_le_bytes());
        format[8..12].copy_from_slice(&(rate * u32::from(alignment)).to_le_bytes());
        format[12..14].copy_from_slice(&alignment.to_le_bytes());
        format[14..16].copy_from_slice(&bits.to_le_bytes());

        let initialize: extern "system" fn(
            Interface,
            u32,
            u32,
            i64,
            i64,
            *const c_void,
            *const Guid,
        ) -> HResult = client.method(3);
        let result = initialize(
            client.0,
            SHARED_MODE,
            LOOPBACK | AUTO_CONVERT_PCM | DEFAULT_SRC_QUALITY,
            2_000_000,
            0,
            format.as_ptr().cast(),
            null(),
        );
        check(result, "Starting shared-mode loopback")?;

        let mut capture = null_mut();
        let get_service: extern "system" fn(Interface, *const Guid, *mut Interface) -> HResult =
            client.method(14);
        let result = get_service(client.0, &CAPTURE_CLIENT, &mut capture);
        check(result, "Opening the loopback stream")?;
        let capture = Com(capture);

        let start: extern "system" fn(Interface) -> HResult = client.method(10);
        let stop: extern "system" fn(Interface) -> HResult = client.method(11);
        check(start(client.0), "Starting the playback recording")?;
        eprintln!("Recording Windows playback for {seconds} seconds…");
        let samples = read_packets(&capture, seconds, rate, alignment as usize);
        let stopped = stop(client.0);
        let samples = samples?;
        check(stopped, "Stopping the playback recording")?;
        if samples.is_empty() {
            return Err("The playback device returned no audio. Play sound on that device during recording.".into());
        }

        let duration = samples.len() as f64 / f64::from(alignment) / f64::from(rate);
        let data_size = samples.len() as u32;
        let padding = data_size % 2;
        let mut bytes = Vec::with_capacity(44 + samples.len() + padding as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_size + padding).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&format[..16]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        bytes.extend_from_slice(&samples);
        if padding != 0 {
            bytes.push(0);
        }

        Ok(Recording {
            bytes,
            duration,
            rate,
            channels,
        })
    }
}

unsafe fn read_packets(
    capture: &Com,
    seconds: f64,
    rate: u32,
    alignment: usize,
) -> Result<Vec<u8>> {
    unsafe {
        let next_packet: extern "system" fn(Interface, *mut u32) -> HResult = capture.method(5);
        let get_buffer: extern "system" fn(
            Interface,
            *mut *const u8,
            *mut u32,
            *mut u32,
            *mut u64,
            *mut u64,
        ) -> HResult = capture.method(3);
        let release_buffer: extern "system" fn(Interface, u32) -> HResult = capture.method(4);
        let capacity = (seconds * f64::from(rate)).round() as usize * alignment;
        let mut samples = Vec::with_capacity(capacity);
        let deadline = Instant::now() + Duration::from_secs_f64(seconds);
        loop {
            let mut pending = 0;
            check(
                next_packet(capture.0, &mut pending),
                "Checking the loopback stream",
            )?;
            if pending != 0 {
                let mut data = null();
                let mut frames = 0;
                let mut flags = 0;
                let result = get_buffer(
                    capture.0,
                    &mut data,
                    &mut frames,
                    &mut flags,
                    null_mut(),
                    null_mut(),
                );
                check(result, "Reading the loopback stream")?;
                let bytes = (frames as usize * alignment).min(capacity - samples.len());
                if flags & SILENT_PACKET != 0 {
                    samples.resize(samples.len() + bytes, 0);
                } else if bytes != 0 {
                    samples.extend_from_slice(std::slice::from_raw_parts(data, bytes));
                }
                check(
                    release_buffer(capture.0, frames),
                    "Releasing the loopback packet",
                )?;
            }

            if samples.len() == capacity || (pending == 0 && Instant::now() >= deadline) {
                return Ok(samples);
            }
            if pending == 0 {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
