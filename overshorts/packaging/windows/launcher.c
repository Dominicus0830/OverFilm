/* OverShorts.exe: runs the bundled Python with app\launch.py.
 *
 * Built as a GUI program (no console window). Double-clicked with no arguments it starts
 * python.exe with CREATE_NO_WINDOW: Python and every tool it runs (ffmpeg, node, claude) share
 * one console that has no window, so nothing flashes on screen. launch.py writes the output to
 * %LOCALAPPDATA%\OverShorts\logs\overshorts.log and opens the OverShorts window (serve --window).
 * With arguments (command-line use) it attaches to the calling console and shows the output there.
 *
 * Build (mingw-w64):
 *   x86_64-w64-mingw32-windres res.rc -O coff -o res.o
 *   x86_64-w64-mingw32-gcc -O2 -municode -mwindows -o OverShorts.exe launcher.c res.o
 */
#include <windows.h>
#include <wchar.h>

/* launch.py exits with this code after it already showed its own message box. */
#define EXIT_ALREADY_TOLD 3

static const wchar_t *skip_argv0(const wchar_t *cmd) {
    if (*cmd == L'"') {
        cmd++;
        while (*cmd && *cmd != L'"') cmd++;
        if (*cmd) cmd++;
    } else {
        while (*cmd && *cmd != L' ' && *cmd != L'\t') cmd++;
    }
    while (*cmd == L' ' || *cmd == L'\t') cmd++;
    return cmd;
}

int WINAPI wWinMain(HINSTANCE inst, HINSTANCE prev, PWSTR cmdline, int show) {
    (void)inst; (void)prev; (void)cmdline; (void)show;
    wchar_t dir[MAX_PATH];
    DWORD n = GetModuleFileNameW(NULL, dir, MAX_PATH);
    if (n == 0 || n >= MAX_PATH) return 1;
    wchar_t *slash = wcsrchr(dir, L'\\');
    if (!slash) return 1;
    *slash = 0;
    const wchar_t *rest = skip_argv0(GetCommandLineW());
    BOOL cli = *rest != 0;

    static wchar_t cmd[32768];
    int len = _snwprintf(cmd, 32767, L"\"%ls\\python\\python.exe\" -X utf8 \"%ls\\app\\launch.py\" %ls", dir, dir, rest);
    if (len < 0) return 1;
    cmd[32767] = 0;
    wchar_t exe[MAX_PATH + 32];
    _snwprintf(exe, MAX_PATH + 31, L"%ls\\python\\python.exe", dir);
    exe[MAX_PATH + 31] = 0;

    STARTUPINFOW si;
    PROCESS_INFORMATION pi;
    ZeroMemory(&si, sizeof si);
    si.cb = sizeof si;
    ZeroMemory(&pi, sizeof pi);
    DWORD flags = CREATE_NO_WINDOW;
    BOOL inherit = FALSE;
    if (cli && AttachConsole(ATTACH_PARENT_PROCESS)) {
        /* command-line use: show the output in the console we were started from */
        si.dwFlags = STARTF_USESTDHANDLES;
        si.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
        si.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
        si.hStdError = GetStdHandle(STD_ERROR_HANDLE);
        flags = 0;
        inherit = TRUE;
    }
    if (!CreateProcessW(exe, cmd, NULL, NULL, inherit, flags, NULL, dir, &si, &pi)) {
        MessageBoxW(NULL, L"OverShorts를 시작하지 못했습니다. 설치 폴더의 python 폴더가 있는지 확인해 주세요.",
                    L"OverShorts", MB_ICONERROR);
        return 1;
    }
    WaitForSingleObject(pi.hProcess, INFINITE);
    DWORD code = 1;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    if (code != 0 && code != EXIT_ALREADY_TOLD && !cli) {
        wchar_t msg[1024];
        _snwprintf(msg, 1023,
                   L"OverShorts가 오류로 끝났습니다 (코드 %lu).\n\n"
                   L"자세한 내용은 로그 파일에 있습니다:\n%%LOCALAPPDATA%%\\OverShorts\\logs\\overshorts.log",
                   code);
        msg[1023] = 0;
        MessageBoxW(NULL, msg, L"OverShorts", MB_ICONERROR);
    }
    return (int)code;
}
