namespace ExpressionFixture {
    public class Counter {
        public int value;
        public Counter(int start) => value = start;
        public int Double => value * 2;
        public int Value { get => value; set => this.value = value + 1; }
        public void Add(int delta) => value += delta;
        public int Plus(int x) => value + x;
    }
    public static class Runner {
        public static int Run() {
            Counter c = new Counter(4);
            c.Add(3);
            c.Value = 9;
            return c.Double + c.Plus(2) + c.Value;
        }
    }
}
