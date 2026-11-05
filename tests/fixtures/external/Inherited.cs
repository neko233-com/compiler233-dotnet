namespace Demo
{
    public class MyError : System.Exception
    {
        public MyError(string message) : base(message)
        {
        }

        public int Code()
        {
            return 7;
        }
    }

    public static class ErrorRunner
    {
        public static int Run()
        {
            MyError error = new MyError("boom");
            if (error.Message == "boom")
            {
                return error.Code();
            }
            return 0;
        }
    }
}
