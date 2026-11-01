//! Hand-written bindings for the few CoreFoundation / CoreMedia / CoreVideo / VideoToolbox / AudioToolbox calls the
//! native video player needs (macOS and iOS share them). No crate wraps these in the versions we have locked, and
//! the surface is small enough to declare by hand.

#![allow(non_snake_case, non_upper_case_globals, non_camel_case_types, dead_code)]

use std::ffi::c_void;

pub type OSStatus = i32;
pub type CFTypeRef = *const c_void;
pub type CFAllocatorRef = *const c_void;
pub type CFDictionaryRef = *const c_void;
pub type CMFormatDescriptionRef = *const c_void;
pub type CMBlockBufferRef = *const c_void;
pub type CMSampleBufferRef = *const c_void;
pub type CVImageBufferRef = *const c_void;
pub type VTDecompressionSessionRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

impl CMTime {
    pub const VALID: u32 = 1;
    pub const INVALID: CMTime = CMTime { value: 0, timescale: 0, flags: 0, epoch: 0 };

    pub fn micros(us: i64) -> CMTime {
        CMTime { value: us, timescale: 1_000_000, flags: Self::VALID, epoch: 0 }
    }

    /// Microseconds, or 0 for an invalid time.
    pub fn as_micros(&self) -> i64 {
        if self.flags & Self::VALID == 0 || self.timescale <= 0 {
            return 0;
        }
        (self.value as i128 * 1_000_000 / self.timescale as i128) as i64
    }
}

#[repr(C)]
pub struct CMSampleTimingInfo {
    pub duration: CMTime,
    pub presentation_time_stamp: CMTime,
    pub decode_time_stamp: CMTime,
}

pub type VTDecompressionOutputCallback = unsafe extern "C" fn(
    refcon: *mut c_void,
    source_frame_refcon: *mut c_void,
    status: OSStatus,
    info_flags: u32,
    image_buffer: CVImageBufferRef,
    presentation_time_stamp: CMTime,
    presentation_duration: CMTime,
);

#[repr(C)]
pub struct VTDecompressionOutputCallbackRecord {
    pub callback: VTDecompressionOutputCallback,
    pub refcon: *mut c_void,
}

pub const kCFNumberSInt32Type: i32 = 3;
pub const kCMBlockBufferAssureMemoryNowFlag: u32 = 1;
/// `'BGRA'`.
pub const kCVPixelFormatType_32BGRA: i32 = 0x4247_5241;
pub const kCVPixelBufferLock_ReadOnly: u64 = 1;

/// `kVTInvalidSessionErr`: the system took the session away (iOS does this when the app was in the background).
pub const kVTInvalidSessionErr: OSStatus = -12903;
/// `kVTVideoDecoderBadDataErr`.
pub const kVTVideoDecoderBadDataErr: OSStatus = -12909;
pub const kVTVideoDecoderMalfunctionErr: OSStatus = -12911;
pub const kVTVideoDecoderUnsupportedDataFormatErr: OSStatus = -12910;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub static kCFTypeDictionaryKeyCallBacks: u8;
    pub static kCFTypeDictionaryValueCallBacks: u8;
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFNumberCreate(allocator: CFAllocatorRef, the_type: i32, value: *const c_void) -> CFTypeRef;
    pub fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFDictionaryRef;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    pub fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: CFAllocatorRef,
        parameter_set_count: usize,
        parameter_set_pointers: *const *const u8,
        parameter_set_sizes: *const usize,
        nal_unit_header_length: i32,
        format_description_out: *mut CMFormatDescriptionRef,
    ) -> OSStatus;
    pub fn CMBlockBufferCreateWithMemoryBlock(
        structure_allocator: CFAllocatorRef,
        memory_block: *mut c_void,
        block_length: usize,
        block_allocator: CFAllocatorRef,
        custom_block_source: *const c_void,
        offset_to_data: usize,
        data_length: usize,
        flags: u32,
        block_buffer_out: *mut CMBlockBufferRef,
    ) -> OSStatus;
    pub fn CMBlockBufferReplaceDataBytes(
        source_bytes: *const c_void,
        destination_buffer: CMBlockBufferRef,
        offset_into_destination: usize,
        data_length: usize,
    ) -> OSStatus;
    pub fn CMSampleBufferCreateReady(
        allocator: CFAllocatorRef,
        data_buffer: CMBlockBufferRef,
        format_description: CMFormatDescriptionRef,
        num_samples: isize,
        num_sample_timing_entries: isize,
        sample_timing_array: *const CMSampleTimingInfo,
        num_sample_size_entries: isize,
        sample_size_array: *const usize,
        sample_buffer_out: *mut CMSampleBufferRef,
    ) -> OSStatus;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    pub static kCVPixelBufferPixelFormatTypeKey: CFTypeRef;
    pub fn CVPixelBufferLockBaseAddress(buffer: CVImageBufferRef, flags: u64) -> i32;
    pub fn CVPixelBufferUnlockBaseAddress(buffer: CVImageBufferRef, flags: u64) -> i32;
    pub fn CVPixelBufferGetBaseAddress(buffer: CVImageBufferRef) -> *const u8;
    pub fn CVPixelBufferGetBytesPerRow(buffer: CVImageBufferRef) -> usize;
    pub fn CVPixelBufferGetWidth(buffer: CVImageBufferRef) -> usize;
    pub fn CVPixelBufferGetHeight(buffer: CVImageBufferRef) -> usize;
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    pub fn VTDecompressionSessionCreate(
        allocator: CFAllocatorRef,
        format_description: CMFormatDescriptionRef,
        decoder_specification: CFDictionaryRef,
        image_buffer_attributes: CFDictionaryRef,
        output_callback: *const VTDecompressionOutputCallbackRecord,
        session_out: *mut VTDecompressionSessionRef,
    ) -> OSStatus;
    pub fn VTDecompressionSessionDecodeFrame(
        session: VTDecompressionSessionRef,
        sample_buffer: CMSampleBufferRef,
        decode_flags: u32,
        source_frame_refcon: *mut c_void,
        info_flags_out: *mut u32,
    ) -> OSStatus;
    pub fn VTDecompressionSessionFinishDelayedFrames(session: VTDecompressionSessionRef) -> OSStatus;
    pub fn VTDecompressionSessionWaitForAsynchronousFrames(session: VTDecompressionSessionRef) -> OSStatus;
    pub fn VTDecompressionSessionInvalidate(session: VTDecompressionSessionRef);
}

// ---- AudioToolbox -------------------------------------------------------------------------------------------

pub type AudioConverterRef = *mut c_void;
pub type AudioComponent = *mut c_void;
pub type AudioUnit = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AudioStreamBasicDescription {
    pub sample_rate: f64,
    pub format_id: u32,
    pub format_flags: u32,
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub bytes_per_frame: u32,
    pub channels_per_frame: u32,
    pub bits_per_channel: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioBuffer {
    pub number_channels: u32,
    pub data_byte_size: u32,
    pub data: *mut c_void,
}

/// One buffer only (interleaved formats).
#[repr(C)]
pub struct AudioBufferList {
    pub number_buffers: u32,
    pub buffers: [AudioBuffer; 1],
}

#[repr(C)]
pub struct AudioStreamPacketDescription {
    pub start_offset: i64,
    pub variable_frames_in_packet: u32,
    pub data_byte_size: u32,
}

#[repr(C)]
pub struct SMPTETime {
    pub subframes: i16,
    pub subframe_divisor: i16,
    pub counter: u32,
    pub kind: u32,
    pub flags: u32,
    pub hours: i16,
    pub minutes: i16,
    pub seconds: i16,
    pub frames: i16,
}

#[repr(C)]
pub struct AudioTimeStamp {
    pub sample_time: f64,
    pub host_time: u64,
    pub rate_scalar: f64,
    pub word_clock_time: u64,
    pub smpte_time: SMPTETime,
    pub flags: u32,
    pub reserved: u32,
}

pub const kAudioTimeStampHostTimeValid: u32 = 1 << 1;

#[repr(C)]
pub struct AudioComponentDescription {
    pub component_type: u32,
    pub component_sub_type: u32,
    pub component_manufacturer: u32,
    pub component_flags: u32,
    pub component_flags_mask: u32,
}

pub type AURenderCallback = unsafe extern "C" fn(
    refcon: *mut c_void,
    action_flags: *mut u32,
    time_stamp: *const AudioTimeStamp,
    bus_number: u32,
    number_frames: u32,
    io_data: *mut AudioBufferList,
) -> OSStatus;

#[repr(C)]
pub struct AURenderCallbackStruct {
    pub input_proc: AURenderCallback,
    pub input_proc_ref_con: *mut c_void,
}

pub type AudioConverterComplexInputDataProc = unsafe extern "C" fn(
    converter: AudioConverterRef,
    io_number_data_packets: *mut u32,
    io_data: *mut AudioBufferList,
    out_data_packet_description: *mut *mut AudioStreamPacketDescription,
    user_data: *mut c_void,
) -> OSStatus;

/// FourCC from ASCII.
pub const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
}

pub const kAudioFormatLinearPCM: u32 = fourcc(b"lpcm");
pub const kAudioFormatMPEG4AAC: u32 = fourcc(b"aac ");
pub const kAudioFormatFlagIsFloat: u32 = 1;
pub const kAudioFormatFlagIsPacked: u32 = 8;
pub const kAudioConverterDecompressionMagicCookie: u32 = fourcc(b"dmgc");
pub const kAudioUnitType_Output: u32 = fourcc(b"auou");
pub const kAudioUnitManufacturer_Apple: u32 = fourcc(b"appl");
#[cfg(target_os = "macos")]
pub const kAudioUnitSubType_Output: u32 = fourcc(b"def ");
#[cfg(not(target_os = "macos"))]
pub const kAudioUnitSubType_Output: u32 = fourcc(b"rioc");
pub const kAudioUnitProperty_StreamFormat: u32 = 8;
pub const kAudioUnitProperty_SetRenderCallback: u32 = 23;
pub const kAudioUnitScope_Input: u32 = 1;

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    pub fn AudioConverterNew(
        source: *const AudioStreamBasicDescription,
        destination: *const AudioStreamBasicDescription,
        converter_out: *mut AudioConverterRef,
    ) -> OSStatus;
    pub fn AudioConverterDispose(converter: AudioConverterRef) -> OSStatus;
    pub fn AudioConverterReset(converter: AudioConverterRef) -> OSStatus;
    pub fn AudioConverterSetProperty(converter: AudioConverterRef, property: u32, size: u32, data: *const c_void) -> OSStatus;
    pub fn AudioConverterFillComplexBuffer(
        converter: AudioConverterRef,
        input_data_proc: AudioConverterComplexInputDataProc,
        input_data_proc_user_data: *mut c_void,
        io_output_data_packet_size: *mut u32,
        out_output_data: *mut AudioBufferList,
        out_packet_description: *mut AudioStreamPacketDescription,
    ) -> OSStatus;

    pub fn AudioComponentFindNext(component: AudioComponent, desc: *const AudioComponentDescription) -> AudioComponent;
    pub fn AudioComponentInstanceNew(component: AudioComponent, instance_out: *mut AudioUnit) -> OSStatus;
    pub fn AudioComponentInstanceDispose(instance: AudioUnit) -> OSStatus;
    pub fn AudioUnitSetProperty(unit: AudioUnit, property: u32, scope: u32, element: u32, data: *const c_void, size: u32) -> OSStatus;
    pub fn AudioUnitInitialize(unit: AudioUnit) -> OSStatus;
    pub fn AudioUnitUninitialize(unit: AudioUnit) -> OSStatus;
    pub fn AudioOutputUnitStart(unit: AudioUnit) -> OSStatus;
    pub fn AudioOutputUnitStop(unit: AudioUnit) -> OSStatus;
}

#[repr(C)]
struct MachTimebase {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebase) -> i32;
}

/// Monotonic host clock in mach ticks (what `AudioTimeStamp::host_time` uses).
pub fn host_ticks() -> u64 {
    unsafe { mach_absolute_time() }
}

/// Converts a signed span of mach ticks to microseconds.
pub fn ticks_to_us(ticks: i64) -> f64 {
    use std::sync::OnceLock;
    static BASE: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *BASE.get_or_init(|| {
        let mut info = MachTimebase { numer: 1, denom: 1 };
        unsafe { mach_timebase_info(&mut info) };
        (info.numer.max(1), info.denom.max(1))
    });
    ticks as f64 * numer as f64 / denom as f64 / 1000.
}
