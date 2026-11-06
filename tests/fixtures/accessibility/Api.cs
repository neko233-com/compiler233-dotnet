namespace AccessFixture {
    public interface IApi { int Run(); }
    public class Api {
        int DefaultField;
        private int PrivateField;
        internal int InternalField;
        protected int ProtectedField;
        protected internal int ProtectedInternalField;
        private protected int PrivateProtectedField;
        public int PublicField;
        public static int StaticField;
        int DefaultMethod() { return DefaultField; }
        private int PrivateMethod() { return PrivateField; }
        internal int InternalMethod() { return InternalField; }
        protected int ProtectedMethod() { return ProtectedField; }
        protected internal int ProtectedInternalMethod() { return ProtectedInternalField; }
        private protected int PrivateProtectedMethod() { return PrivateProtectedField; }
        public int PublicMethod() { return PublicField; }
        public static int StaticMethod() { return 1; }
        public int Value { get; private set; }
        public int Write { private get; set; }
        protected internal int Guarded { get; protected set; }
    }
    class DefaultType { public int Field; }
    internal class InternalType { public int Field; }
    public class ExplicitConstructor { ExplicitConstructor(int n) {} }
}
