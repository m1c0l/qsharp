/// # Sample
/// Adaptive profile optimizations
///
/// # Description
/// A few code snippets that are optimized by the Q# compiler when it
/// generates QIR.

import Std.Convert.IntAsDouble;
import Std.Math.ArcSin, Std.Math.ArcCos, Std.Math.PI, Std.Math.Sin;

@EntryPoint()
operation Main() : Result[] {
    use q = Qubit();

    // This gets optimized to a series of gates with constant argument.
    for idx in 0..3 {
        if idx % 2 == 0 {
            Rz(ArcSin(1.) + IntAsDouble(idx) * PI(), q);
        } else {
            Rz(ArcCos(-1.) + IntAsDouble(idx) * PI(), q);
        }
    }

    // This gets optimized to constant calls within branches.
    use controls = Qubit[2];
    ApplyToEachCA(H, controls);
    use targets = Qubit[2];
    if MResetZ(controls[0]) != MResetZ(controls[1]) {
        let angle = PI() + PI() + PI() * Sin(PI() / 2.0);
        Rxx(angle, targets[0], targets[1]);
    } else {
        Rxx(PI() + PI() + 2.0 * PI() * Sin(PI() / 2.0), targets[0], targets[1]);
    }

    let qRes = MResetZ(q);
    let controlsRes = MResetEachZ(controls);
    let targetsRes = MResetEachZ(targets);
    [qRes] + controlsRes + targetsRes
}
