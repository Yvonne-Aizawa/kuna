//! (kuna `ctrsvc`) Nintendo 3DS supervisor calls.
//!
//! The 3DS kernel takes the call number from the `svc` immediate, not from a
//! register, reads its arguments from `r0`..`r5`, returns the result code in `r0`
//! and writes any further outputs to `r1`..`r5`. With the option on, each known
//! number becomes a call to a user-op named after its libctru wrapper, reading the
//! registers the kernel takes and writing `r0` plus the outputs the wrapper
//! stores. The table comes from libctru's wrapper stubs, so the registers read
//! are the ones the kernel takes, not the ones the function happens to set. An
//! unknown number is left as the bare `software_interrupt`.

use super::*;
use crate::userop::{BUILTIN_SVC_BASE, BUILTIN_SVC_OUT};

/// Generated from libctru's `svc.s` stubs and `svc.h`: `(number, wrapper, outputs, inputs)`; `outputs` has bit k-1 set when the call writes `rk` besides `r0` and `inputs` has bit k set when the kernel reads `rk`.
pub(crate) const SVCS: &[(u8, &str, u8, u8)] = &[
    (0x01, "svcControlMemory", 0x01, 0x1f),
    (0x02, "svcQueryMemory", 0x1f, 0x04),
    (0x03, "svcExitProcess", 0x00, 0x00),
    (0x04, "svcGetProcessAffinityMask", 0x00, 0x07),
    (0x05, "svcSetProcessAffinityMask", 0x00, 0x07),
    (0x06, "svcGetProcessIdealProcessor", 0x01, 0x02),
    (0x07, "svcSetProcessIdealProcessor", 0x00, 0x03),
    (0x08, "svcCreateThread", 0x01, 0x1f),
    (0x09, "svcExitThread", 0x00, 0x00),
    (0x0a, "svcSleepThread", 0x00, 0x03),
    (0x0b, "svcGetThreadPriority", 0x01, 0x02),
    (0x0c, "svcSetThreadPriority", 0x00, 0x03),
    (0x0d, "svcGetThreadAffinityMask", 0x00, 0x07),
    (0x0e, "svcSetThreadAffinityMask", 0x00, 0x07),
    (0x0f, "svcGetThreadIdealProcessor", 0x01, 0x02),
    (0x10, "svcSetThreadIdealProcessor", 0x00, 0x03),
    (0x11, "svcGetProcessorID", 0x00, 0x00),
    (0x12, "svcRun", 0x00, 0x3f),
    (0x13, "svcCreateMutex", 0x01, 0x02),
    (0x14, "svcReleaseMutex", 0x00, 0x01),
    (0x15, "svcCreateSemaphore", 0x01, 0x06),
    (0x16, "svcReleaseSemaphore", 0x01, 0x06),
    (0x17, "svcCreateEvent", 0x01, 0x02),
    (0x18, "svcSignalEvent", 0x00, 0x01),
    (0x19, "svcClearEvent", 0x00, 0x01),
    (0x1a, "svcCreateTimer", 0x01, 0x02),
    (0x1b, "svcSetTimer", 0x00, 0x1f),
    (0x1c, "svcCancelTimer", 0x00, 0x01),
    (0x1d, "svcClearTimer", 0x00, 0x01),
    (0x1e, "svcCreateMemoryBlock", 0x01, 0x0f),
    (0x1f, "svcMapMemoryBlock", 0x00, 0x0f),
    (0x20, "svcUnmapMemoryBlock", 0x00, 0x03),
    (0x21, "svcCreateAddressArbiter", 0x01, 0x00),
    (0x22, "svcArbitrateAddress", 0x00, 0x3f),
    (0x23, "svcCloseHandle", 0x00, 0x01),
    (0x24, "svcWaitSynchronization", 0x00, 0x0d),
    (0x25, "svcWaitSynchronizationN", 0x01, 0x1f),
    (0x27, "svcDuplicateHandle", 0x01, 0x02),
    (0x28, "svcGetSystemTick", 0x01, 0x00),
    (0x29, "svcGetHandleInfo", 0x03, 0x06),
    (0x2a, "svcGetSystemInfo", 0x03, 0x06),
    (0x2b, "svcGetProcessInfo", 0x03, 0x06),
    (0x2c, "svcGetThreadInfo", 0x03, 0x06),
    (0x2d, "svcConnectToPort", 0x01, 0x02),
    (0x32, "svcSendSyncRequest", 0x00, 0x01),
    (0x33, "svcOpenProcess", 0x01, 0x02),
    (0x34, "svcOpenThread", 0x01, 0x06),
    (0x35, "svcGetProcessId", 0x01, 0x02),
    (0x36, "svcGetProcessIdOfThread", 0x01, 0x02),
    (0x37, "svcGetThreadId", 0x01, 0x02),
    (0x38, "svcGetResourceLimit", 0x01, 0x02),
    (0x39, "svcGetResourceLimitLimitValues", 0x00, 0x0f),
    (0x3a, "svcGetResourceLimitCurrentValues", 0x00, 0x0f),
    (0x3c, "svcBreak", 0x00, 0x01),
    (0x3d, "svcOutputDebugString", 0x00, 0x03),
    (0x3e, "svcControlPerformanceCounter", 0x03, 0x0f),
    (0x47, "svcCreatePort", 0x03, 0x0c),
    (0x48, "svcCreateSessionToPort", 0x01, 0x02),
    (0x49, "svcCreateSession", 0x03, 0x00),
    (0x4a, "svcAcceptSession", 0x01, 0x02),
    (0x4f, "svcReplyAndReceive", 0x01, 0x0e),
    (0x50, "svcBindInterrupt", 0x00, 0x0f),
    (0x51, "svcUnbindInterrupt", 0x00, 0x03),
    (0x52, "svcInvalidateProcessDataCache", 0x00, 0x07),
    (0x53, "svcStoreProcessDataCache", 0x00, 0x07),
    (0x54, "svcFlushProcessDataCache", 0x00, 0x07),
    (0x55, "svcStartInterProcessDma", 0x01, 0x3f),
    (0x56, "svcStopDma", 0x00, 0x01),
    (0x57, "svcGetDmaState", 0x01, 0x02),
    (0x58, "svcRestartDma", 0x00, 0x1f),
    (0x59, "svcSetGpuProt", 0x00, 0x01),
    (0x5a, "svcSetWifiEnabled", 0x00, 0x01),
    (0x60, "svcDebugActiveProcess", 0x01, 0x02),
    (0x61, "svcBreakDebugProcess", 0x00, 0x01),
    (0x62, "svcTerminateDebugProcess", 0x00, 0x01),
    (0x63, "svcGetProcessDebugEvent", 0x00, 0x03),
    (0x64, "svcContinueDebugEvent", 0x00, 0x03),
    (0x65, "svcGetProcessList", 0x01, 0x06),
    (0x66, "svcGetThreadList", 0x01, 0x0e),
    (0x67, "svcGetDebugThreadContext", 0x00, 0x0f),
    (0x68, "svcSetDebugThreadContext", 0x00, 0x0f),
    (0x69, "svcQueryDebugProcessMemory", 0x1f, 0x0c),
    (0x6a, "svcReadProcessMemory", 0x00, 0x0f),
    (0x6b, "svcWriteProcessMemory", 0x00, 0x0f),
    (0x6c, "svcSetHardwareBreakPoint", 0x00, 0x07),
    (0x6d, "svcGetDebugThreadParam", 0x07, 0x0d),
    (0x70, "svcControlProcessMemory", 0x00, 0x3f),
    (0x71, "svcMapProcessMemory", 0x00, 0x07),
    (0x72, "svcUnmapProcessMemory", 0x00, 0x07),
    (0x73, "svcCreateCodeSet", 0x01, 0x0f),
    (0x75, "svcCreateProcess", 0x01, 0x0e),
    (0x76, "svcTerminateProcess", 0x00, 0x01),
    (0x77, "svcSetProcessResourceLimits", 0x00, 0x03),
    (0x78, "svcCreateResourceLimit", 0x01, 0x00),
    (0x79, "svcSetResourceLimitValues", 0x00, 0x0f),
    (0x7b, "svcBackdoor", 0x00, 0x00),
    (0x7c, "svcKernelSetState", 0x00, 0x03),
    (0x7d, "svcQueryProcessMemory", 0x1f, 0x0c),
];

/// The table row for supervisor call `number`.
pub(super) fn entry(number: u64) -> Option<(u8, &'static str, u8, u8)> {
    let number = u8::try_from(number).ok()?;
    SVCS.iter().find(|row| row.0 == number).copied()
}

/// The libctru wrapper name behind a supervisor-call built-in id.
pub fn builtin_name(id: uint4) -> Option<&'static str> {
    let number = u8::try_from(id.checked_sub(BUILTIN_SVC_BASE)?).ok()?;
    entry(u64::from(number)).map(|row| row.1)
}

/// Every supervisor-call built-in id the table defines.
pub fn builtin_ids() -> impl Iterator<Item = uint4> {
    SVCS.iter().map(|row| BUILTIN_SVC_BASE + uint4::from(row.0))
}

/// Is `id` one of the supervisor-call built-ins?
pub(super) fn is_builtin(id: uint4) -> bool {
    builtin_name(id).is_some()
}

/// The supervisor-call number of a bare `software_interrupt` op.
pub(super) fn immediate(data: &Funcdata, op: OpId) -> Option<u64> {
    let o = data.obank().get(op)?;
    let vn = data.vbank().get(o.get_in(1)?)?;
    vn.is_constant().then(|| vn.get_offset())
}

/// Rename `op` to its wrapper and give it the registers the kernel reads and writes.
pub(super) fn rewrite(
    data: &mut Funcdata,
    op: OpId,
    regs: &SyscallStorage,
    number: u8,
    outputs: u8,
    inputs: u8,
) -> KunaResult<()> {
    let reads: Vec<(Address, int4)> = (0..6)
        .filter(|k| inputs & (1 << k) != 0)
        .map(|k| regs.inputs[1 + k].clone())
        .collect();
    let id = data.new_constant(4, u64::from(BUILTIN_SVC_BASE + u32::from(number)));
    data.op_set_input(op, id, 0)?;
    data.op_remove_input(op, 1);
    for (i, (addr, size)) in reads.iter().enumerate() {
        let vn = data.new_varnode(*size, addr, None);
        data.op_insert_input(op, vn, 1 + i as int4)?;
    }
    data.new_varnode_out(regs.result.1, &regs.result.0, op)?;
    let extra: Vec<(Address, int4)> = {
        let lookup = data.get_arch().manage().register_lookup();
        (1..=5u8)
            .filter(|k| outputs & (1 << (k - 1)) != 0)
            .filter_map(|k| {
                let st = lookup.and_then(|l| l.probe_register(&format!("r{k}")))?;
                Some((Address::new(st.space.clone()?, st.offset), st.size as int4))
            })
            .collect()
    };
    for (addr, size) in &extra {
        define_output(data, op, addr, *size)?;
    }
    Ok(())
}

fn define_output(data: &mut Funcdata, op: OpId, addr: &Address, size: int4) -> KunaResult<()> {
    let pc = data
        .obank()
        .get(op)
        .map(|o| o.get_addr().clone())
        .ok_or_else(|| KunaError::lowlevel("ctrsvc: stale supervisor call"))?;
    let out = data.new_op(1, pc);
    data.op_set_opcode_code(out, OpCode::CPUI_CALLOTHER);
    if let Some(o) = data.obank_mut().get_mut(out) {
        o.clear_flag(pcodeop_flags::call);
    }
    let id = data.new_constant(4, u64::from(BUILTIN_SVC_OUT));
    data.op_set_input(out, id, 0)?;
    data.new_varnode_out(size, addr, out)?;
    data.op_insert_after(out, op);
    Ok(())
}
