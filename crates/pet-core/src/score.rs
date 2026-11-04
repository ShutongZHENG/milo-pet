//! 点击积分：飞行中的宠物被点击时按速度与体型算分，速度越快、宠物越小分越高。
//! 对标 dsh-pet `src/shared/score.ts`（阈值与公式逐字一致）。

/// 达标速度阈值（px/s，逻辑）：飞行中被点击时低于它不给分（低速点击维持普通点击动画）。
pub const SCORE_MIN_SPEED: f64 = 400.0;

/// 每 100 px/s 记 1 分（基准尺寸 462px 下）。
const SCORE_SPEED_PER_POINT: f64 = 100.0;

/// 基准宠物宽度（dsh constants.ts PET_REF_WIDTH）。
const PET_REF_WIDTH: f64 = 462.0;

/// 点击积分：score = round(speed/100 × 462/size)，至少 1 分。
/// 小宠物目标小、飞行中更难命中，反而分高；speed ≤ 0 或 size ≤ 0 返回 0。
pub fn click_score(speed: f64, size: f64) -> i32 {
    if speed <= 0.0 || size <= 0.0 {
        return 0;
    }
    ((speed / SCORE_SPEED_PER_POINT) * (PET_REF_WIDTH / size)).round().max(1.0) as i32
}
