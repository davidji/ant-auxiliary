use crate::{
    proto::{ 
        FanRequest, 
        FanRequest_, 
        FanResponse, 
        Response, 
        Response_::Peripheral as ResponsePeripheral,
    },
    shell,
};

use defmt::{ debug, info, warn };
use stm32f4xx_hal::{
    pac::TIM3,
    timer::PwmChannel, 
};

use rtic_sync::signal::SignalReader;

use crate::fugit::Rate;

pub struct Fan<'a> {
    pub pwm: shell::TaskResponses<PwmChannel<TIM3, 0>>,
    pub freq_reader: SignalReader<'a, crate::Duration>,
}

impl <'a> Fan<'a> {
    pub async fn process(&mut self, request: FanRequest) {
       match request {
            FanRequest { command: Some(FanRequest_::Command::Set(set)) } => {
                info!("fan set duty {}", set.duty);
                self.pwm.task.set_duty(set.duty as u16);
            },
            FanRequest { command: Some(FanRequest_::Command::Get(_)) } => { },
            FanRequest { command: _ } => {
                warn!("Unknown command for fan");
            }
        }

        let response = self.response();
        self.pwm.responses.send(response).await.unwrap();
    }

    fn response(&mut self) -> Response {
        Response { peripheral: Some(ResponsePeripheral::Fan(FanResponse {
            duty: self.pwm.task.get_duty() as i32,
            rpm: self.rpm(),
        })) }
    }

    fn rpm(&mut self) -> i32 {
        match self.freq_reader.try_read() {
            Some(duration) => {
                debug!("fan pulse duration: {}", duration);
                let rate: Rate<u64, 1, 1> = duration.into_rate();
                debug!("fan rate {}", rate);
                // convert to RPM (*60), the pulse rate is double the rotation rate (/2)
                30 * (rate.to_Hz() as i32)
            },
            None => {
                debug!("No fan pulse duration");
                *FanResponse::default().rpm()
            },
        }
    }
}


