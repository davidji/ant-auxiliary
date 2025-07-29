use crate::{
    proto::{ 
        FanRequest, 
        FanRequest_, 
        FanResponse, 
        Response, 
        Response_::Peripheral as ResponsePeripheral,
    },
    shell::ResponseSender,
};

use defmt::{ debug, info, warn };
use embedded_hal::pwm::SetDutyCycle;

use rtic_sync::signal::SignalReader;

use crate::fugit::Rate;

pub struct Fan<'a, PWM: SetDutyCycle> {
    pwm: PWM,
    responses: ResponseSender,
    freq_reader: SignalReader<'a, crate::Duration>,
    curent_duty: f32,
}

impl <'a, PWM: SetDutyCycle> Fan<'a, PWM> {
    pub fn new(
        pwm: PWM,
        responses: ResponseSender,
        freq_reader: SignalReader<'a, crate::Duration>,
    ) -> Self {
        Fan {
            pwm,
            responses,
            freq_reader,
            curent_duty: 0.0,
        }
    }

    pub async fn process(&mut self, request: FanRequest) {
       match request {
            FanRequest { command: Some(FanRequest_::Command::Set(set)) } => {
                info!("fan set duty {}", set.duty);
                self.pwm.set_duty_cycle((set.duty*self.pwm.max_duty_cycle() as f32) as u16).unwrap();
                self.curent_duty = set.duty;
            },
            FanRequest { command: Some(FanRequest_::Command::Get(_)) } => { },
            FanRequest { command: _ } => {
                warn!("Unknown command for fan");
            }
        }

        let response = self.response();
        self.responses.send(response).await.unwrap();
    }

    fn response(&mut self) -> Response {
        Response { peripheral: Some(ResponsePeripheral::Fan(FanResponse {
            duty: self.curent_duty,
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


