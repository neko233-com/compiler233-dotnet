using P = StructFixture.Point;
namespace StructConsumer {
    public static class AliasRunner {
        public static int Run() {
            P point = default(P);
            point.X = 17;
            return point.X;
        }
    }
}
