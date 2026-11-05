namespace Demo
{
    public static class ListRunner
    {
        public static int Run()
        {
            System.Collections.Generic.List<int> items = new System.Collections.Generic.List<int>();
            items.Add(3);
            items.Add(4);
            return items.Count;
        }
    }
}
