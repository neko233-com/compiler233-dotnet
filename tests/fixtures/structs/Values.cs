namespace StructFixture {
    public static class Runner {
        public static int Trace;
        public static int Next(int value) { Trace = Trace * 10 + value; return value; }

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

        public static int MixedCopy() {
            Mixed original = default(Mixed);
            if (original.Enabled || original.Name != null || original.Token != null || original.Data != null) return -1;
            original.Count = 2;
            original.Enabled = true;
            original.Ratio = 2.5f;
            original.Name = "before";
            original.Token = "shared";
            original.Data = new int[1];
            original.Data[0] = 3;
            original.State = State.Ready;
            Mixed copy = original;
            copy.Count = 9;
            copy.Name = "after";
            copy.Data[0] = 7;
            copy.Internal = 4;
            if (!original.Enabled || original.Ratio != 2.5f || original.Name != "before" ||
                copy.Name != "after" || original.State != State.Ready || original.Token == null) return -2;
            return original.Count * 1000 + copy.Count * 100 + original.Data[0] * 10 + copy.Internal;
        }

        public static Point InitializedPoint() {
            return new Point { X = Next(1), Y = Next(2) };
        }

        public static int InitializerOrder() {
            Trace = 0;
            Point point = InitializedPoint();
            Mixed mixed = new Mixed() { Count = point.X, Enabled = true, Name = "ready", State = State.Ready };
            if (!mixed.Enabled || mixed.Name != "ready" || mixed.State != State.Ready || mixed.Count != 1) return -1;
            return Trace * 100 + point.X * 10 + point.Y;
        }
    }

    public class Box { public Point Value; }
    public static class Shared { public static Point Value; }
    public enum State { None, Ready }
    public struct Point {
        public int X;
        public int Y;
    }
    public struct Mixed {
        public int Count;
        public bool Enabled;
        public float Ratio;
        public string Name;
        public object Token;
        public int[] Data;
        public State State;
        internal int Internal;
        private int Secret;
        int Hidden;
    }
}
