//! 帧包加载：assets/frames.bin（预乘 BGRA，zstd 压缩），见 tools/pack_frames.py。

use std::fs;

const MAGIC: &[u8; 8] = b"MPETFB01";

pub struct FramePack {
    pub count: u32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    offsets: Vec<u32>,
    blob_section: usize,
    file: Vec<u8>,
}

impl FramePack {
    pub fn load(path: &str) -> Result<Self, String> {
        let file = fs::read(path).map_err(|e| format!("读取帧包失败 {path}: {e}"))?;
        if file.len() < 24 || &file[0..8] != MAGIC {
            return Err("帧包格式错误（magic 不匹配）".into());
        }
        let u32_at = |p: usize| u32::from_le_bytes(file[p..p + 4].try_into().unwrap());
        let count = u32_at(8);
        let width = u32_at(12);
        let height = u32_at(16);
        let fps = u32_at(20);
        let mut offsets = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            offsets.push(u32_at(24 + i * 4));
        }
        let blob_section = 24 + count as usize * 4;
        Ok(FramePack { count, width, height, fps, offsets, blob_section, file })
    }

    pub fn frame_bytes(&self) -> usize {
        (self.width * self.height * 4) as usize
    }

    /// 解压第 i 帧到 out（长度必须为 width*height*4）。
    pub fn decode_frame(&self, i: usize, out: &mut Vec<u8>) -> Result<(), String> {
        let off = *self.offsets.get(i).ok_or("帧索引越界")? as usize;
        let len = u32::from_le_bytes(
            self.file[off..off + 4].try_into().map_err(|_| "帧包损坏")?,
        ) as usize;
        let start = off + 4;
        let compressed = &self.file[start..start + len];
        zstd::bulk::decompress(compressed, self.frame_bytes())
            .map(|decoded| {
                out.clear();
                out.extend_from_slice(&decoded);
            })
            .map_err(|e| format!("帧 {i} 解压失败: {e}"))
    }
}
