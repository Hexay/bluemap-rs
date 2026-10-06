import de.bluecolored.bluemap.core.util.math.*;

// Rotation.init() transcribed onto BlueMap's real MatrixM4f/VectorM3f; prints Rust test constants.
public class ModelRef {
    static String f(float v) { return String.format("0x%08x", Float.floatToRawIntBits(v)); }
    static String m4(MatrixM4f m) {
        float[] a = {m.m00, m.m01, m.m02, m.m03, m.m10, m.m11, m.m12, m.m13, m.m20, m.m21, m.m22, m.m23, m.m30, m.m31, m.m32, m.m33};
        StringBuilder s = new StringBuilder("[");
        for (int i = 0; i < a.length; i++) { if (i > 0) s.append(", "); s.append(f(a[i])); }
        return s.append("]").toString();
    }

    static MatrixM4f rotation(float ox, float oy, float oz, float x, float y, float z, boolean rescale) {
        MatrixM4f matrix = new MatrixM4f();
        if (x != 0 || y != 0 || z != 0) {
            matrix.translate(-ox, -oy, -oz);
            matrix.rotateYXZ(x, y, z);
            if (rescale) {
                VectorM3f v = new VectorM3f(0, 0, 0);
                float sX = 1f / v.set(1, 0, 0).rotateAndScale(matrix).absolute().max();
                float sY = 1f / v.set(0, 1, 0).rotateAndScale(matrix).absolute().max();
                float sZ = 1f / v.set(0, 0, 1).rotateAndScale(matrix).absolute().max();
                matrix.identity();
                matrix.translate(-ox, -oy, -oz);
                matrix.scale(sX, sY, sZ);
                matrix.rotateYXZ(x, y, z);
            }
            matrix.translate(ox, oy, oz);
        }
        return matrix;
    }

    static void row(String name, float ox, float oy, float oz, float x, float y, float z, boolean rescale) {
        MatrixM4f r = rotation(ox, oy, oz, x, y, z, rescale);
        MatrixM4f t = new MatrixM4f().copy(r).scale(1f / 16f, 1f / 16f, 1f / 16f);
        System.out.println("    // " + name);
        System.out.println("    RotationRef { origin: [" + ox + ", " + oy + ", " + oz + "], xyz: [" + x + ", " + y + ", " + z + "], rescale: " + rescale + ",");
        System.out.println("        matrix: " + m4(r) + ",");
        System.out.println("        transform: " + m4(t) + " },");
    }

    public static void main(String[] args) {
        System.out.println("pub const ROTATIONS: &[RotationRef] = &[");
        row("zero", 8, 8, 8, 0, 0, 0, false);
        row("axis y 45", 8, 8, 8, 0, 45, 0, false);
        row("axis y 45 rescale", 8, 8, 8, 0, 45, 0, true);
        row("axis x 22.5 rescale, origin 0", 0, 0, 0, 22.5f, 0, 0, true);
        row("axis z -22.5", 8, 4.5f, 3.25f, 0, 0, -22.5f, false);
        row("axis z -45 rescale", 8, 8, 8, 0, 0, -45, true);
        row("xyz 22.5 45 0", 8, 8, 8, 22.5f, 45, 0, false);
        row("xyz 10 -30 15 rescale", 1, 2, 3, 10, -30, 15, true);
        System.out.println("];");
    }
}
