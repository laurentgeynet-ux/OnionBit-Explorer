# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  §B-59 — plafond de commit imposé par l'OS (Job Object).

  Un watchdog par polling ne peut jamais rattraper une alloc
  unique `with_capacity`/`Pixmap`/`String` de 2³⁵ octets : la
  réservation sature le commit en un appel, avant toute mesure.
  `JOB_OBJECT_LIMIT_JOB_MEMORY` fait refuser la requête *dans*
  l'allocateur : Rust aborte le daemon seul, la station reste
  stable — et `RUST_BACKTRACE=1` imprime alors le site exact.

  Outil de vol uniquement : aucun unsafe dans le code livré.
.EXAMPLE
  pwsh -File scripts/vol_joblimit.ps1 -TargetPid 14820 -LimitMB 5120
#>
param(
    [Parameter(Mandatory = $true)][int]$TargetPid,
    [Parameter(Mandatory = $true)][long]$LimitMB
)

$src = @"
using System;
using System.Runtime.InteropServices;

public static class JobLimit {
    [StructLayout(LayoutKind.Sequential)]
    struct IO_COUNTERS {
        public ulong ReadOperationCount; public ulong WriteOperationCount;
        public ulong OtherOperationCount; public ulong ReadTransferCount;
        public ulong WriteTransferCount; public ulong OtherTransferCount;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct BASIC {
        public long PerProcessUserTimeLimit; public long PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize; public UIntPtr MaximumWorkingSetSize;
        public uint ActiveProcessLimit; public UIntPtr Affinity;
        public uint PriorityClass; public uint SchedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct EXTENDED {
        public BASIC BasicLimitInformation; public IO_COUNTERS IoInfo;
        public UIntPtr ProcessMemoryLimit; public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed; public UIntPtr PeakJobMemoryUsed;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateJobObject(IntPtr attr, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetInformationJobObject(IntPtr job, int cls, IntPtr info, uint len);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AssignProcessToJobObject(IntPtr job, IntPtr proc);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr h);

    const int JobObjectExtendedLimitInformation = 9;
    const uint JOB_OBJECT_LIMIT_JOB_MEMORY = 0x00000200;
    const uint PROCESS_SET_QUOTA = 0x0100;
    const uint PROCESS_TERMINATE = 0x0001;

    public static string Apply(int pid, long limitBytes) {
        IntPtr job = CreateJobObject(IntPtr.Zero, null);
        if (job == IntPtr.Zero)
            return "CreateJobObject err=" + Marshal.GetLastWin32Error();
        var ext = new EXTENDED();
        ext.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_JOB_MEMORY;
        ext.JobMemoryLimit = (UIntPtr)limitBytes;
        IntPtr p = Marshal.AllocHGlobal(Marshal.SizeOf(ext));
        Marshal.StructureToPtr(ext, p, false);
        bool ok = SetInformationJobObject(job, JobObjectExtendedLimitInformation, p,
            (uint)Marshal.SizeOf(ext));
        Marshal.FreeHGlobal(p);
        if (!ok) { CloseHandle(job); return "SetInformationJobObject err=" + Marshal.GetLastWin32Error(); }
        IntPtr proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid);
        if (proc == IntPtr.Zero) { CloseHandle(job); return "OpenProcess err=" + Marshal.GetLastWin32Error(); }
        ok = AssignProcessToJobObject(job, proc);
        int err = Marshal.GetLastWin32Error();
        CloseHandle(proc);
        if (!ok) { CloseHandle(job); return "AssignProcessToJobObject err=" + err; }
        // Le handle reste ouvert : le job (et son plafond) vit
        // tant que ce processus hôte vit — fermé à la sortie.
        return "OK job=" + job.ToInt64() + " limit=" + limitBytes;
    }
}
"@

Add-Type -TypeDefinition $src -Language CSharp
$result = [JobLimit]::Apply($TargetPid, $LimitMB * 1MB)
Write-Output "joblimit pid=$TargetPid limitMB=$LimitMB -> $result"
# Garde le handle du job vivant en arrière-plan : tant que ce
# pwsh tourne, le plafond tient. La boucle rend la main au
# script de vol quand le daemon meurt.
while (Get-Process -Id $TargetPid -ErrorAction SilentlyContinue) {
    Start-Sleep -Seconds 2
}
