class Arithmetic {
    static int Add(int a, int b) { return a + b; }
    static int Scale(int a, int b) { return (a + b) * 10 - 3; }
    static int Divide(int a, int b) { return a / (b + 1); }
    static int DivByConstantZero(int a) { return a / 0; }
    static int Constant() { return 100 / (3 + 2); }
    static int Overload(int a) { return a * 2; }
    static int Overload(int a, int b) { return a - b; }
}
