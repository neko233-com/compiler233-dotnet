namespace StructFixture {
    public static class Runner {
        public static int ChangeCopy(Point point) {
            point.X = 9;
            return point.X;
        }

        public static Point Make() {
            Point point = new Point();
            point.X = 3;
            point.Y = 4;
            return point;
        }

        public static int Run() {
            Point original = default(Point);
            original.X = 1;
            original.Y = 2;
            Point copy = original;
            copy.X = 7;
            int changed = ChangeCopy(original);
            Point made = Make();
            return original.X * 10000 + original.Y * 1000 + copy.X * 100 + changed * 10 + made.X + made.Y;
        }

        public static int ContainerCopy() {
            Box box = new Box();
            box.Value = Make();
            Shared.Value = box.Value;
            Point copy = Shared.Value;
            copy.X = 8;
            return box.Value.X * 100 + copy.X * 10 + Shared.Value.Y;
        }
    }

    public class Box { public Point Value; }
    public static class Shared { public static Point Value; }
    public struct Point {
        public int X;
        public int Y;
    }
}
