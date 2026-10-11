//! Sonde de calage : applique un curseur à quelques couleurs et imprime le résultat.
//! Sert à comparer le portage Rust au modèle ajusté, et à voir ce que chaque curseur fait.
use engine::adjust::Adjustment;

fn main() {
    let colours = [[0.45, 0.50, 0.60], [0.62, 0.48, 0.40], [0.05, 0.20, 0.95], [0.90, 0.10, 0.85], [0.58; 3], [0.12; 3], [0.88; 3]];
    for (name, index, value) in [
        ("vibrance +100", 10, 100.0),
        ("saturation +100", 11, 100.0),
        ("vibrance -100", 10, -100.0),
        ("exposition +1 IL", 2, 1.0),
        ("ombres +100", 5, 100.0),
        ("blancs +100", 6, 100.0),
    ] {
        let mut v = [0.0f32; 15];
        v[index] = value;
        let prepared = develop(v).prepare();
        print!("{name:17}");
        for colour in colours {
            let o = prepared.apply(colour);
            print!(" | {:3.0} {:3.0} {:3.0}", o[0] * 255.0, o[1] * 255.0, o[2] * 255.0);
        }
        println!();
    }
    print!("{:17}", "entrée");
    for c in colours {
        print!(" | {:3.0} {:3.0} {:3.0}", c[0] * 255.0, c[1] * 255.0, c[2] * 255.0);
    }
    println!();
}

fn develop(v: [f32; 15]) -> Adjustment {
    Adjustment::Develop {
        temperature: v[0], tint: v[1], exposure: v[2], contrast: v[3], highlights: v[4], shadows: v[5],
        whites: v[6], blacks: v[7], clarity: v[8], dehaze: v[9], vibrance: v[10], saturation: v[11],
        red_hue: v[12], green_hue: v[13], blue_hue: v[14],
    }
}
