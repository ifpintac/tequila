Attribute VB_Name = "SendToTequila"
' Send to Tequila: exports the active SOLIDWORKS part or assembly as STEP AP214,
' with its appearances, and hands the file to Tequila.
' Late bound (no references needed), so the same text also works as a .swb macro.
' Entry point: main (kept as the last procedure in the module).
Option Explicit

#If VBA7 Then
Private Declare PtrSafe Sub Sleep Lib "kernel32" (ByVal dwMilliseconds As Long)
#Else
Private Declare Sub Sleep Lib "kernel32" (ByVal dwMilliseconds As Long)
#End If

' swDocumentTypes_e
Private Const SW_DOC_PART As Long = 1
Private Const SW_DOC_ASSEMBLY As Long = 2
Private Const SW_DOC_DRAWING As Long = 3
' swSaveAsVersion_e.swSaveAsCurrentVersion, swSaveAsOptions_e.swSaveAsOptions_Silent
Private Const SW_SAVE_AS_CURRENT_VERSION As Long = 0
Private Const SW_SAVE_AS_SILENT As Long = 1
' swUserPreferenceIntegerValue_e.swStepAP (value 203 or 214) and .swStepExportPreference
Private Const SW_STEP_AP As Long = 75
Private Const SW_STEP_EXPORT_PREFERENCE As Long = 142
' swAcisOutputGeometryPreference_e.swAcisOutputAsSolidAndSurface
Private Const SW_OUTPUT_SOLID_AND_SURFACE As Long = 0
' swUserPreferenceToggle_e.swStepExportAtomicSave and .swStepExportAppearances
Private Const SW_STEP_EXPORT_ATOMIC_SAVE As Long = 786
Private Const SW_STEP_EXPORT_APPEARANCES As Long = 787

Private Const APP_TITLE As String = "Send to Tequila"
Private Const UNINSTALL_VALUE As String = "\Software\Microsoft\Windows\CurrentVersion\Uninstall\{EDD88596-57CD-4475-98DD-4D3B719E8AB3}_is1\InstallLocation"
Private Const EXE_NAME As String = "Tequila.exe"
Private Const SEND_TIMEOUT_MS As Long = 300000

Dim swApp As Object

' Tequila.exe from the installer's uninstall entry (per-user, then all users),
' else the default install folders; "" when none is found.
Private Function FindTequila() As String
    Dim shell As Object, fso As Object
    Set shell = CreateObject("WScript.Shell")
    Set fso = CreateObject("Scripting.FileSystemObject")

    Dim places(3) As String
    places(0) = RegistryFolder(shell, "HKCU" & UNINSTALL_VALUE)
    places(1) = RegistryFolder(shell, "HKLM" & UNINSTALL_VALUE)
    places(2) = Environ$("LOCALAPPDATA") & "\Programs\Tequila\"
    places(3) = Environ$("ProgramFiles") & "\Tequila\"

    Dim i As Long
    For i = 0 To 3
        If places(i) <> "" Then
            If fso.FileExists(places(i) & EXE_NAME) Then
                FindTequila = places(i) & EXE_NAME
                Exit Function
            End If
        End If
    Next
End Function

' A folder read from the registry, with a trailing backslash; "" when the key or value is missing.
Private Function RegistryFolder(ByVal shell As Object, ByVal valueName As String) As String
    Dim folder As String
    On Error Resume Next
    folder = CStr(shell.RegRead(valueName)) ' raises an error when the key or value does not exist
    If Err.Number <> 0 Then Exit Function
    On Error GoTo 0
    If folder <> "" And Right$(folder, 1) <> "\" Then folder = folder & "\"
    RegistryFolder = folder
End Function

' Creates a folder and any missing parents.
Private Sub MakeFolder(ByVal fso As Object, ByVal folder As String)
    If folder = "" Then Exit Sub
    If fso.FolderExists(folder) Then Exit Sub
    MakeFolder fso, fso.GetParentFolderName(folder)
    On Error Resume Next
    fso.CreateFolder folder
End Sub

' The document's file name without extension (its title if never saved), made safe for a file name.
Private Function ModelName(ByVal swModel As Object, ByVal fso As Object) As String
    Dim baseName As String
    If swModel.GetPathName <> "" Then
        baseName = fso.GetBaseName(swModel.GetPathName)
    Else
        ' GetTitle includes ".SLDPRT" only when Explorer shows known file extensions.
        baseName = swModel.GetTitle
        Select Case LCase$(Right$(baseName, 7))
            Case ".sldprt", ".sldasm"
                baseName = Left$(baseName, Len(baseName) - 7)
        End Select
    End If

    Dim c As Variant
    For Each c In Array("\", "/", ":", "*", "?", """", "<", ">", "|")
        baseName = Replace(baseName, c, "_")
    Next
    baseName = Trim$(baseName)
    If baseName = "" Then baseName = "Model"
    ModelName = baseName
End Function

' Sets the STEP options Tequila needs and saves; "" on success, otherwise the reason.
Private Function SaveStep(ByVal swModel As Object, ByVal stepPath As String, ByRef errs As Long, ByRef warns As Long) As String
    On Error GoTo Failed
    swApp.SetUserPreferenceIntegerValue SW_STEP_AP, 214 ' AP214 carries colours; AP203 has none
    swApp.SetUserPreferenceIntegerValue SW_STEP_EXPORT_PREFERENCE, SW_OUTPUT_SOLID_AND_SURFACE
    swApp.SetUserPreferenceToggle SW_STEP_EXPORT_APPEARANCES, True
    swApp.SetUserPreferenceToggle SW_STEP_EXPORT_ATOMIC_SAVE, False ' one file, with the assembly structure

    ' With faces or bodies selected, SaveAs exports only those.
    swModel.ClearSelection2 True

    Dim ext As Object, saved As Boolean
    Set ext = swModel.Extension
    On Error Resume Next
    saved = ext.SaveAs3(stepPath, SW_SAVE_AS_CURRENT_VERSION, SW_SAVE_AS_SILENT, Nothing, Nothing, errs, warns)
    If Err.Number = 438 Then
        ' SaveAs3 arrived in SOLIDWORKS 2020 SP02; older versions have SaveAs.
        Err.Clear
        On Error GoTo Failed
        saved = ext.SaveAs(stepPath, SW_SAVE_AS_CURRENT_VERSION, SW_SAVE_AS_SILENT, Nothing, errs, warns)
    ElseIf Err.Number <> 0 Then
        SaveStep = Err.Description
        Exit Function
    End If
    If Not saved Then SaveStep = "SOLIDWORKS could not save it."
    Exit Function
Failed:
    SaveStep = Err.Description
End Function

' Exports to %TEMP%\Tequila\SolidWorks\<name>.step; the path, or "" after telling the user why not.
Private Function ExportStep(ByVal swModel As Object) As String
    Dim fso As Object
    Set fso = CreateObject("Scripting.FileSystemObject")

    Dim folder As String
    folder = Environ$("TEMP") & "\Tequila\SolidWorks"
    MakeFolder fso, folder
    If Not fso.FolderExists(folder) Then
        MsgBox "Could not create the folder " & folder, vbCritical, APP_TITLE
        Exit Function
    End If

    Dim stepPath As String
    stepPath = folder & "\" & ModelName(swModel, fso) & ".step"

    ' Remove the last export first, so a failed export cannot leave an old file for Tequila to load.
    On Error Resume Next
    If fso.FileExists(stepPath) Then fso.DeleteFile stepPath, True
    On Error GoTo 0
    If fso.FileExists(stepPath) Then
        MsgBox "Could not replace " & stepPath & vbCrLf & "Is it open in another program?", vbExclamation, APP_TITLE
        Exit Function
    End If

    ' The user's own STEP options: system options that persist, so they are put back afterwards.
    Dim oldAP As Long, oldOutput As Long, oldAppearances As Boolean, oldAtomic As Boolean
    oldAP = swApp.GetUserPreferenceIntegerValue(SW_STEP_AP)
    oldOutput = swApp.GetUserPreferenceIntegerValue(SW_STEP_EXPORT_PREFERENCE)
    oldAppearances = swApp.GetUserPreferenceToggle(SW_STEP_EXPORT_APPEARANCES)
    oldAtomic = swApp.GetUserPreferenceToggle(SW_STEP_EXPORT_ATOMIC_SAVE)

    Dim pathBefore As String
    pathBefore = swModel.GetPathName

    Dim errs As Long, warns As Long, failure As String
    failure = SaveStep(swModel, stepPath, errs, warns)

    swApp.SetUserPreferenceIntegerValue SW_STEP_AP, oldAP
    swApp.SetUserPreferenceIntegerValue SW_STEP_EXPORT_PREFERENCE, oldOutput
    swApp.SetUserPreferenceToggle SW_STEP_EXPORT_APPEARANCES, oldAppearances
    swApp.SetUserPreferenceToggle SW_STEP_EXPORT_ATOMIC_SAVE, oldAtomic

    If failure = "" And Not fso.FileExists(stepPath) Then failure = "SOLIDWORKS reported success but wrote no file."
    If failure <> "" Then
        MsgBox "Could not export the STEP file." & vbCrLf & failure & vbCrLf & _
               "(swFileSaveError_e " & errs & ", swFileSaveWarning_e " & warns & ")", vbExclamation, APP_TITLE
        Exit Function
    End If
    If swModel.GetPathName <> pathBefore Then
        ' Not expected for an export; here so that a first test run would show it.
        MsgBox "Note: SOLIDWORKS now shows this document as " & swModel.GetPathName, vbInformation, APP_TITLE
    End If
    ExportStep = stepPath
End Function

' Runs Tequila.exe --send "<file>" and waits for it. It returns as soon as an
' open Tequila has the model, or as soon as it has started one with it, so
' the wait is only long for a very large assembly.
Private Sub HandToTequila(ByVal exePath As String, ByVal stepPath As String)
    Dim shell As Object
    Set shell = CreateObject("WScript.Shell")
    Dim job As Object, waited As Long
    Set job = shell.Exec("""" & exePath & """ --send """ & stepPath & """")
    Do While job.Status = 0 And waited < SEND_TIMEOUT_MS
        Sleep 100
        DoEvents
        waited = waited + 100
    Loop

    If job.Status = 0 Then
        MsgBox "Tequila is still taking in the model. It will appear when it is ready.", vbInformation, APP_TITLE
    ElseIf job.ExitCode = 0 Then
        swApp.Frame.SetStatusBarText "Sent to Tequila: " & stepPath
    Else
        Dim detail As String
        If Not job.StdErr.AtEndOfStream Then detail = Trim$(job.StdErr.ReadAll)
        MsgBox "Tequila could not take the model (exit code " & job.ExitCode & ")." & vbCrLf & detail, _
               vbExclamation, APP_TITLE
    End If
End Sub

Sub main()
    Set swApp = Application.SldWorks

    Dim swModel As Object
    Set swModel = swApp.ActiveDoc
    If swModel Is Nothing Then
        MsgBox "Open a part or an assembly, then run Send to Tequila again.", vbInformation, APP_TITLE
        Exit Sub
    End If

    Select Case swModel.GetType
        Case SW_DOC_PART, SW_DOC_ASSEMBLY
            ' Fine.
        Case SW_DOC_DRAWING
            MsgBox "Tequila renders parts and assemblies, not drawings." & vbCrLf & _
                   "Open the part or assembly and send that.", vbExclamation, APP_TITLE
            Exit Sub
        Case Else
            MsgBox "Only parts and assemblies can be sent to Tequila.", vbExclamation, APP_TITLE
            Exit Sub
    End Select

    Dim exePath As String
    exePath = FindTequila()
    If exePath = "" Then
        MsgBox "Tequila is not installed for this Windows user." & vbCrLf & _
               "Install it from https://github.com/ifpintac/tequila/releases, then try again.", _
               vbExclamation, APP_TITLE
        Exit Sub
    End If

    Dim stepPath As String
    stepPath = ExportStep(swModel)
    If stepPath <> "" Then HandToTequila exePath, stepPath
End Sub
