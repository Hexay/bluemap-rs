use bm_java::JavaRandom;

const SQRT_3: f64 = 1.7320508075688772;
const F2: f64 = 0.5 * (SQRT_3 - 1.0);
const G2: f64 = (3.0 - SQRT_3) / 6.0;

const GRADIENT: [[i32; 3]; 12] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
];

/// BlueMap's 2D simplex noise (vanilla's `SimplexNoise`); swamp grass uses one seeded with `JavaRandom::new(2345)`.
#[derive(Clone, Debug)]
pub struct SimplexNoise {
    p: [u8; 256],
}

impl SimplexNoise {
    pub fn new(random: &mut JavaRandom) -> Self {
        // vanilla draws its x/y/z origin offsets first; BlueMap discards them but must consume them
        for _ in 0..3 {
            random.next_double();
        }
        let mut p: [u8; 256] = std::array::from_fn(|i| i as u8);
        for i in 0..256 {
            let offset = random.next_int(256 - i as i32) as usize;
            p.swap(i, i + offset);
        }
        Self { p }
    }

    pub fn permutation(&self) -> &[u8; 256] {
        &self.p
    }

    fn p(&self, index: i32) -> i32 {
        self.p[(index & 255) as usize] as i32
    }

    pub fn get_value(&self, x: f64, y: f64) -> f64 {
        let skew = (x + y) * F2;
        let i = (x + skew).floor() as i32;
        let j = (y + skew).floor() as i32;

        let unskew = i.wrapping_add(j) as f64 * G2;
        let x0 = x - (i as f64 - unskew);
        let y0 = y - (j as f64 - unskew);

        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };

        let x1 = x0 - i1 as f64 + G2;
        let y1 = y0 - j1 as f64 + G2;
        let x2 = x0 - 1.0 + 2.0 * G2;
        let y2 = y0 - 1.0 + 2.0 * G2;

        let ii = i & 255;
        let jj = j & 255;

        let gi0 = self.p(ii + self.p(jj)) % 12;
        let gi1 = self.p(ii + i1 + self.p(jj + j1)) % 12;
        let gi2 = self.p(ii + 1 + self.p(jj + 1)) % 12;

        let n0 = corner_noise(gi0, x0, y0, 0.0, 0.5);
        let n1 = corner_noise(gi1, x1, y1, 0.0, 0.5);
        let n2 = corner_noise(gi2, x2, y2, 0.0, 0.5);

        70.0 * (n0 + n1 + n2)
    }
}

fn corner_noise(index: i32, x: f64, y: f64, z: f64, base: f64) -> f64 {
    let mut t = base - x * x - y * y - z * z;
    if t < 0.0 {
        return 0.0;
    }
    t *= t;
    let g = GRADIENT[index as usize];
    t * t * (g[0] as f64 * x + g[1] as f64 * y + g[2] as f64 * z)
}
