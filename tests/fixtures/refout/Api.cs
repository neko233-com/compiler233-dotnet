namespace Demo {
    public interface ICounter {
        int Twice(int value);
    }
    public class Counter : ICounter {
        public int Value { get; set; }
        public int[] Values { get; set; }
        public static int Offset { get; set; }
        public int Twice(int value) { return value + value; }
        public static string ImplementationOnly() { return "IMPLEMENTATION_ONLY_4A27"; }
    }
}
