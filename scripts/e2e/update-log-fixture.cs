// The test builds a failing updater child and an associated .log reader from
// this fixture. No test hooks are compiled into the production application.
using System;
using System.IO;
using System.Reflection;
using System.Text;

internal static class UpdateLogFixture
{
    private static int Main(string[] args)
    {
        string executable = Assembly.GetExecutingAssembly().Location;
        string directory = Path.GetDirectoryName(executable);
        if (Path.GetFileName(executable).Equals("powershell.exe", StringComparison.OrdinalIgnoreCase))
        {
            int code = Int32.Parse(File.ReadAllText(Path.Combine(directory, "result-code.txt")));
            if (code != 0 && code != 10)
            {
                Console.Error.WriteLine("E2E updater stderr sentinel: failure before helper logging");
                Console.Error.Write(new string('x', 65536));
                Console.Error.WriteLine("END-SHOULD-NOT-BE-CAPTURED");
            }
            // The normal PowerShell helper cleans its temporary script directory.
            int scriptIndex = Array.IndexOf(args, "-File");
            if (scriptIndex >= 0 && scriptIndex + 1 < args.Length)
            {
                string scriptDirectory = Path.GetDirectoryName(args[scriptIndex + 1]);
                if (Path.GetFileName(scriptDirectory).StartsWith("brightwheel-update-", StringComparison.Ordinal))
                    Directory.Delete(scriptDirectory, true);
            }
            return code;
        }

        string record;
        try
        {
            if (args.Length != 1) throw new InvalidOperationException("Expected exactly one document argument");
            record = Convert.ToBase64String(Encoding.UTF8.GetBytes(args[0])) + "\t" +
                Convert.ToBase64String(Encoding.UTF8.GetBytes(File.ReadAllText(args[0])));
        }
        catch (Exception error)
        {
            record = "ERROR: " + error.Message;
        }
        File.AppendAllText(Path.Combine(directory, "opened-paths.txt"), record + Environment.NewLine);
        return record.StartsWith("ERROR:", StringComparison.Ordinal) ? 1 : 0;
    }
}
