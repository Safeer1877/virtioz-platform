use futures_util::StreamExt;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[tokio::main]
async fn main() {
    let mut cmd = Command::new("powershell.exe");
    cmd.arg("-NoProfile").arg("-Command").arg("echo 'Hello GitHub'");
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    
    match cmd.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();
            
            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr).lines();
            
            let out_task = tokio::spawn(async move {
                while let Ok(Some(line)) = stdout_reader.next_line().await {
                    println!("OUT: {}", line);
                }
            });
            
            let err_task = tokio::spawn(async move {
                while let Ok(Some(line)) = stderr_reader.next_line().await {
                    println!("ERR: {}", line);
                }
            });
            
            let _ = out_task.await;
            let _ = err_task.await;
            let status = child.wait().await.unwrap();
            println!("Finished with code {:?}", status.code());
        }
        Err(e) => {
            println!("Failed to spawn: {}", e);
        }
    }
}
