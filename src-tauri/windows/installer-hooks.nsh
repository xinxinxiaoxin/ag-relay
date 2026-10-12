; ADR-0005：安装器不能替应用强停进程。覆盖 Tauri 默认的强停宏，
; 安装与卸载都只等待本安装目录的面板/Node 退出，预算耗尽即拒绝。
; Tauri 改动模板接缝时编译失败，避免静默退回不安全行为。
!ifmacrondef CheckIfAppIsRunning
  !error "AG Relay: Tauri installer safety seam changed; review the template before bundling."
!endif
!macroundef CheckIfAppIsRunning

!macro CheckIfAppIsRunning executablePath productName
  !define AGRelayCheckID ${__LINE__}
  StrCpy $R9 10
  !insertmacro RestartManager_StartSession $R0
  ${If} $R0 == ""
    SetErrorLevel 2
    Abort "无法确认安装目录的进程占用情况，已取消操作；请稍后重试。"
  ${EndIf}

  ${If} ${FileExists} "${executablePath}"
    !insertmacro RestartManager_RegisterFile $R0 "${executablePath}"
    ${If} $0 != 0
      !insertmacro RestartManager_EndSession $R0
      SetErrorLevel 2
      Abort "无法确认面板是否已退出，已取消操作。"
    ${EndIf}
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\node.exe"
    !insertmacro RestartManager_RegisterFile $R0 "$INSTDIR\node.exe"
    ${If} $0 != 0
      !insertmacro RestartManager_EndSession $R0
      SetErrorLevel 2
      Abort "无法确认受管网关是否已退出，已取消操作。"
    ${EndIf}
  ${EndIf}

  ag_relay_wait_${AGRelayCheckID}:
    System::Call 'RSTRTMGR::RmGetList(p R0, *i .r1, *i .r2, p 0, *i .r3) i .r0'
    ${If} $0 == ${ERROR_MORE_DATA}
      ; updater 启动安装器后才退出面板，允许这个短暂交叠；不发任何停机信号。
      ${If} $R9 > 0
        IntOp $R9 $R9 - 1
        Sleep 500
        Goto ag_relay_wait_${AGRelayCheckID}
      ${EndIf}
      !insertmacro RestartManager_EndSession $R0
      SetErrorLevel 2
      Abort "${productName} 或随包 Node 仍在使用本安装目录，已取消操作。请先安全停止网关并退出面板，再重试。"
    ${ElseIf} $0 != 0
      !insertmacro RestartManager_EndSession $R0
      SetErrorLevel 2
      Abort "读取进程占用状态失败，已取消操作；不会强行结束进程。"
    ${EndIf}
  !insertmacro RestartManager_EndSession $R0
  !undef AGRelayCheckID
!macroend
