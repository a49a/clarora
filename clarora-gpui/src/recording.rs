use anyhow::{Context, Result, bail};
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
type Writer = Arc<Mutex<Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>>>;
pub struct Recorder {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<Result<PathBuf>>>,
}
fn stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    writer: Writer,
    error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
) -> Result<cpal::Stream>
where
    T: SizedSample + Sample,
    i16: FromSample<T>,
{
    let write_error = error.clone();
    let write_stop = stop.clone();
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| {
            if let Ok(mut guard) = writer.lock() {
                if let Some(writer) = guard.as_mut() {
                    for &sample in data {
                        if let Err(e) = writer.write_sample(sample.to_sample::<i16>()) {
                            *write_error.lock().unwrap() = Some(e.to_string());
                            write_stop.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
            }
        },
        move |e| {
            *error.lock().unwrap() = Some(e.to_string());
            stop.store(true, Ordering::SeqCst);
        },
        None,
    )?)
}
impl Recorder {
    pub fn start(path: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = thread::spawn(move || {
            let result = (|| {
                let device = cpal::default_host()
                    .default_input_device()
                    .context("没有麦克风，请检查设备和系统权限")?;
                let config = device.default_input_config()?;
                let writer = Arc::new(Mutex::new(Some(hound::WavWriter::create(
                    &path,
                    hound::WavSpec {
                        channels: config.channels(),
                        sample_rate: config.sample_rate().0,
                        bits_per_sample: 16,
                        sample_format: hound::SampleFormat::Int,
                    },
                )?)));
                let error = Arc::new(Mutex::new(None));
                let output = match config.sample_format() {
                    SampleFormat::F32 => stream::<f32>(
                        &device,
                        &config.into(),
                        writer.clone(),
                        error.clone(),
                        flag.clone(),
                    )?,
                    SampleFormat::I16 => stream::<i16>(
                        &device,
                        &config.into(),
                        writer.clone(),
                        error.clone(),
                        flag.clone(),
                    )?,
                    SampleFormat::U16 => stream::<u16>(
                        &device,
                        &config.into(),
                        writer.clone(),
                        error.clone(),
                        flag.clone(),
                    )?,
                    _ => bail!("暂不支持此麦克风采样格式"),
                };
                output.play()?;
                while !flag.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(50));
                }
                drop(output);
                writer
                    .lock()
                    .unwrap()
                    .take()
                    .context("录音器未启动")?
                    .finalize()?;
                if let Some(message) = error.lock().unwrap().take() {
                    bail!("录音失败：{message}");
                }
                Ok(path.clone())
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&path);
            }
            result
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }
    pub fn finish(mut self) -> Result<PathBuf> {
        self.stop.store(true, Ordering::SeqCst);
        self.worker
            .take()
            .context("录音已停止")?
            .join()
            .map_err(|_| anyhow::anyhow!("录音线程异常"))?
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
