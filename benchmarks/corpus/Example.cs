using System;
using System.Collections.Generic;

namespace Example;
public static class Program
{
    public static int Sum(IEnumerable<int> values)
    {
        int result = 0;
        foreach (int value in values) result += value;
        return result;
    }
    public static void Main() => Console.WriteLine(Sum(new[] { 1, 2, 3 }));
}
