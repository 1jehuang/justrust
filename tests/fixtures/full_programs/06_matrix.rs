const N: usize = 3;

type Matrix = [[i64; N]; N];

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut c = [[0i64; N]; N];
    for i in 0..N {
        for j in 0..N {
            let mut sum = 0;
            for k in 0..N {
                sum += a[i][k] * b[k][j];
            }
            c[i][j] = sum;
        }
    }
    c
}

fn identity() -> Matrix {
    let mut m = [[0; N]; N];
    for i in 0..N {
        m[i][i] = 1;
    }
    m
}

fn power(m: &Matrix, mut e: u32) -> Matrix {
    let mut result = identity();
    let mut base = *m;
    while e > 0 {
        if e & 1 == 1 {
            result = multiply(&result, &base);
        }
        base = multiply(&base, &base);
        e >>= 1;
    }
    result
}

fn print_matrix(m: &Matrix) {
    for row in m.iter() {
        let parts: Vec<String> = row.iter().map(|x| x.to_string()).collect();
        println!("[{}]", parts.join(", "));
    }
}

fn main() {
    let a: Matrix = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];
    let b: Matrix = [[9, 8, 7], [6, 5, 4], [3, 2, 1]];
    print_matrix(&multiply(&a, &b));
    let f: Matrix = [[1, 1, 0], [1, 0, 0], [0, 0, 1]];
    let p = power(&f, 30);
    println!("fib(30) via matrix = {}", p[0][1]);
    let trace: i64 = (0..N).map(|i| a[i][i]).sum();
    println!("trace = {}", trace);
}
