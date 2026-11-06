using Demo;
namespace Client {
    public static class Runner {
        public static int Run(Counter counter) {
            counter.Value = 21;
            return counter.Twice(counter.Value);
        }
    }
}
