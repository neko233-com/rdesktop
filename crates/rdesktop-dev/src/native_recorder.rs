//! Native desktop recording for the development server.
//!
//! Windows uses GDI for a conservative full-desktop capture and Media
//! Foundation's H.264 sink writer for MP4 output. This keeps the recorder in
//! the dev tool, uses codecs already supplied by Windows, and does not spawn
//! or package ffmpeg.

#[cfg(windows)]
mod windows_impl {
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use anyhow::{anyhow, Context};
    use tokio::sync::Mutex;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        HBITMAP, HGDIOBJ, SRCCOPY,
    };
    use windows::Win32::Media::MediaFoundation::{
        IMFMediaType, IMFSinkWriter, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
        MFCreateSinkWriterFromURL, MFMediaType_Video, MFShutdown, MFStartup, MFVideoFormat_H264,
        MFVideoFormat_RGB32, MFVideoInterlace_Progressive, MFSTARTUP_FULL, MF_MT_AVG_BITRATE,
        MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
        MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_VERSION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    struct ActiveRecording {
        stop: Sender<()>,
        join: JoinHandle<anyhow::Result<()>>,
    }

    /// Owns at most one native recording worker.
    pub struct NativeRecorder {
        active: Mutex<Option<ActiveRecording>>,
    }

    impl NativeRecorder {
        pub fn new() -> Self {
            Self {
                active: Mutex::new(None),
            }
        }

        pub async fn start(
            &self,
            path: PathBuf,
            fps: u32,
            max_duration: Duration,
        ) -> anyhow::Result<()> {
            let mut active = self.active.lock().await;
            if active.is_some() {
                anyhow::bail!("native screen recording is already active");
            }

            let (stop_tx, stop_rx) = mpsc::channel();
            let (ready_tx, ready_rx) = mpsc::channel::<()>();
            let join = thread::Builder::new()
                .name("rdesktop-native-recorder".to_string())
                .spawn(move || run_recording(path, fps, max_duration, stop_rx, ready_tx))
                .context("failed to create native recording thread")?;

            tokio::task::spawn_blocking(move || ready_rx.recv())
                .await
                .context("native recording initialization task failed")?
                .map_err(|_| anyhow!("native recording thread exited before initialization"))?;

            *active = Some(ActiveRecording {
                stop: stop_tx,
                join,
            });
            Ok(())
        }

        pub async fn stop(&self) -> anyhow::Result<()> {
            let active = self
                .active
                .lock()
                .await
                .take()
                .ok_or_else(|| anyhow!("native screen recording is not active"))?;
            let _ = active.stop.send(());
            tokio::task::spawn_blocking(move || {
                active
                    .join
                    .join()
                    .map_err(|_| anyhow!("native recording thread panicked"))?
            })
            .await
            .context("native recording stop task failed")?
        }
    }

    fn run_recording(
        path: PathBuf,
        fps: u32,
        max_duration: Duration,
        stop_rx: Receiver<()>,
        ready_tx: Sender<()>,
    ) -> anyhow::Result<()> {
        let result = unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_FULL)
                .context("MFStartup failed")
                .and_then(|_| run_recording_inner(&path, fps, max_duration, &stop_rx, &ready_tx))
        };

        // The worker owns the Media Foundation lifetime. Always shut it down,
        // including when capture or encoder initialization fails.
        let shutdown = unsafe { MFShutdown() }.context("MFShutdown failed");
        match (result, shutdown) {
            (Ok(result), Ok(())) => Ok(result),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    fn run_recording_inner(
        path: &Path,
        fps: u32,
        max_duration: Duration,
        stop_rx: &Receiver<()>,
        ready_tx: &Sender<()>,
    ) -> anyhow::Result<()> {
        let fps = fps.clamp(1, 60);
        let (x, y, width, height) = desktop_rect()?;
        let writer = unsafe { create_writer(path, width, height, fps) }?;
        let stream_index = writer.0;
        let writer = writer.1;

        // Signal readiness only after both the capture dimensions and the MP4
        // sink have been initialized. The server can then report a reliable
        // recording state to the agent.
        let _ = ready_tx.send(());

        let frame_duration = 10_000_000i64 / i64::from(fps);
        let frame_interval = Duration::from_micros(1_000_000 / u64::from(fps));
        let deadline = std::time::Instant::now() + max_duration;
        let mut frame_index = 0u64;

        loop {
            if stop_rx.try_recv().is_ok() || std::time::Instant::now() >= deadline {
                break;
            }

            let pixels = capture_frame(x, y, width, height)?;
            unsafe { write_frame(&writer, stream_index, &pixels, frame_index, frame_duration)? };
            frame_index += 1;

            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match stop_rx.recv_timeout(frame_interval.min(remaining)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }

        unsafe { writer.Finalize() }.context("Media Foundation failed to finalize MP4")?;
        Ok(())
    }

    fn desktop_rect() -> anyhow::Result<(i32, i32, i32, i32)> {
        let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) } & !1;
        let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) } & !1;
        if width <= 0 || height <= 0 {
            anyhow::bail!("Windows returned an invalid virtual desktop size")
        }
        Ok((x, y, width, height))
    }

    fn capture_frame(x: i32, y: i32, width: i32, height: i32) -> anyhow::Result<Vec<u8>> {
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or_else(|| anyhow!("screen dimensions overflowed"))?;
        let byte_count = pixel_count
            .checked_mul(4)
            .ok_or_else(|| anyhow!("screen buffer size overflowed"))?;
        let mut pixels = vec![0u8; byte_count];

        unsafe {
            let screen = GetDC(None);
            if screen.is_invalid() {
                anyhow::bail!("GetDC failed")
            }
            let memory = CreateCompatibleDC(Some(screen));
            if memory.is_invalid() {
                ReleaseDC(None, screen);
                anyhow::bail!("CreateCompatibleDC failed")
            }
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            if bitmap.is_invalid() {
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
                anyhow::bail!("CreateCompatibleBitmap failed")
            }

            let mut selected = false;
            let mut previous_object = HGDIOBJ::default();
            let result = (|| {
                let previous = SelectObject(memory, bitmap.into());
                if previous.is_invalid() {
                    anyhow::bail!("SelectObject failed")
                }
                previous_object = previous;
                selected = true;
                BitBlt(memory, 0, 0, width, height, Some(screen), x, y, SRCCOPY)
                    .context("BitBlt failed")?;
                SelectObject(memory, previous);
                selected = false;

                let mut bitmap_info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: width,
                        biHeight: -height,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        biSizeImage: 0,
                        biXPelsPerMeter: 0,
                        biYPelsPerMeter: 0,
                        biClrUsed: 0,
                        biClrImportant: 0,
                    },
                    bmiColors: [Default::default()],
                };
                let rows = GetDIBits(
                    memory,
                    bitmap,
                    0,
                    height as u32,
                    Some(pixels.as_mut_ptr().cast()),
                    &mut bitmap_info,
                    DIB_RGB_COLORS,
                );
                if rows != height {
                    anyhow::bail!("GetDIBits returned {rows} rows, expected {height}")
                }
                Ok(())
            })();

            if selected {
                // Restore the memory DC before deleting its bitmap, even on
                // an error path.
                SelectObject(memory, previous_object);
            }
            let _ = DeleteObject(HBITMAP::from(bitmap).into());
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            result.map(|_| pixels)
        }
    }

    unsafe fn create_writer(
        path: &Path,
        width: i32,
        height: i32,
        fps: u32,
    ) -> anyhow::Result<(u32, IMFSinkWriter)> {
        let path = path
            .to_str()
            .ok_or_else(|| anyhow!("recording path is not valid UTF-16"))?;
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let writer = MFCreateSinkWriterFromURL(PCWSTR(wide.as_ptr()), None, None)
            .context("MFCreateSinkWriterFromURL failed")?;

        let output_type = media_type(MFVideoFormat_H264, width, height, fps)?;
        output_type
            .SetUINT32(&MF_MT_AVG_BITRATE, 8_000_000)
            .context("failed to set H.264 bitrate")?;
        let stream_index = writer
            .AddStream(&output_type)
            .context("failed to add H.264 output stream")?;

        let input_type = media_type(MFVideoFormat_RGB32, width, height, fps)?;
        writer
            .SetInputMediaType(stream_index, &input_type, None)
            .context("failed to configure RGB32 input")?;
        writer
            .BeginWriting()
            .context("Media Foundation sink writer failed to begin")?;
        Ok((stream_index, writer))
    }

    unsafe fn media_type(
        subtype: windows::core::GUID,
        width: i32,
        height: i32,
        fps: u32,
    ) -> anyhow::Result<IMFMediaType> {
        let media_type = MFCreateMediaType().context("MFCreateMediaType failed")?;
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media_type.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
        media_type.SetUINT64(
            &MF_MT_FRAME_SIZE,
            (u64::try_from(width).unwrap_or_default() << 32)
                | u64::try_from(height).unwrap_or_default(),
        )?;
        media_type.SetUINT64(&MF_MT_FRAME_RATE, u64::from(fps) << 32 | 1)?;
        media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, 1u64 << 32 | 1)?;
        media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        Ok(media_type)
    }

    unsafe fn write_frame(
        writer: &IMFSinkWriter,
        stream_index: u32,
        pixels: &[u8],
        frame_index: u64,
        frame_duration: i64,
    ) -> anyhow::Result<()> {
        let buffer = MFCreateMemoryBuffer(u32::try_from(pixels.len())?)
            .context("MFCreateMemoryBuffer failed")?;
        let mut destination = std::ptr::null_mut();
        buffer
            .Lock(&mut destination, None, None)
            .context("failed to lock Media Foundation buffer")?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), destination, pixels.len());
        buffer
            .Unlock()
            .context("failed to unlock Media Foundation buffer")?;
        buffer.SetCurrentLength(u32::try_from(pixels.len())?)?;

        let sample = MFCreateSample().context("MFCreateSample failed")?;
        sample.AddBuffer(&buffer)?;
        sample.SetSampleTime(i64::try_from(frame_index)?.saturating_mul(frame_duration))?;
        sample.SetSampleDuration(frame_duration)?;
        writer.WriteSample(stream_index, &sample)?;
        Ok(())
    }
}

#[cfg(not(windows))]
mod unsupported_impl {
    use std::{path::PathBuf, time::Duration};

    pub struct NativeRecorder;

    impl NativeRecorder {
        pub fn new() -> Self {
            Self
        }

        pub async fn start(
            &self,
            _path: PathBuf,
            _fps: u32,
            _max_duration: Duration,
        ) -> anyhow::Result<()> {
            anyhow::bail!("native MP4 desktop recording is currently implemented for Windows only")
        }

        pub async fn stop(&self) -> anyhow::Result<()> {
            anyhow::bail!("native MP4 desktop recording is currently implemented for Windows only")
        }
    }
}

#[cfg(not(windows))]
pub use unsupported_impl::NativeRecorder;
#[cfg(windows)]
pub use windows_impl::NativeRecorder;
