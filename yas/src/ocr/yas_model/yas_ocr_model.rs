use std::cell::RefCell;
use std::time::SystemTime;
use image::{EncodableLayout, GrayImage, ImageBuffer, Luma, RgbImage};
use tract_onnx::prelude::*;
use crate::ocr::traits::ImageToText;
use super::preprocess;
use anyhow::Result;
use crate::common::image_ext::*;

type ModelType = RunnableModel<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

pub struct YasOCRModel {
    model: ModelType,
    index_to_word: Vec<String>,

    inference_time: RefCell<f64>,   // in seconds
    invoke_count: RefCell<usize>,
}

impl YasOCRModel {
    fn inc_statistics(&self, time: f64) {
        let mut count_handle = self.invoke_count.borrow_mut();
        *count_handle += 1;

        let mut time_handle = self.inference_time.borrow_mut();
        *time_handle += time;
    }

    pub fn get_average_inference_time(&self) -> f64 {
        let count = *self.invoke_count.borrow();
        let total_time = *self.inference_time.borrow();
        total_time / count as f64
    }

    pub fn new(model: &[u8], content: &str) -> Result<YasOCRModel> {
        let model = tract_onnx::onnx()
            .model_for_read(&mut model.as_bytes())?
            .with_input_fact(0, f32::fact([1, 1, 32, 384]).into())?
            .into_optimized()?
            .into_runnable()?;

        let json = serde_json::from_str::<serde_json::Value>(content)?;

        let mut index_to_word = json
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.parse::<usize>().unwrap(), v.as_str().unwrap().to_string()))
            .collect::<Vec<(usize, String)>>();

        index_to_word.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));

        let index_to_word = index_to_word.into_iter().map(|(_, v)| v).collect();

        Ok(YasOCRModel {
            model,
            index_to_word,
            inference_time: RefCell::new(0.0),
            invoke_count: RefCell::new(0),
        })
    }

    pub fn inference_string(&self, img: &ImageBuffer<Luma<f32>, Vec<f32>>) -> Result<String> {
        let now = SystemTime::now();

        let tensor: Tensor =
            tract_ndarray::Array4::from_shape_fn((1, 1, 32, 384), |(_, _, y, x)| {
                img.get_pixel(x as u32, y as u32)[0]
            }).into();

        let result = self.model.run(tvec!(tensor))?;
        let arr = result[0].to_array_view::<f32>()?;

        let shape = arr.shape();

        let mut ans = String::new();
        let mut last_word = String::new();
        for i in 0..shape[0] {
            let mut max_index = 0;
            let mut max_value = -1.0;
            for j in 0..self.index_to_word.len() {
                let value = arr[[i, 0, j]];
                if value > max_value {
                    max_value = value;
                    max_index = j;
                }
            }
            let word = &self.index_to_word[max_index];
            if *word != last_word && word != "-" {
                ans = ans + word;
            }

            last_word = word.clone();
        }

        let time = now.elapsed()?.as_secs_f64();
        self.inc_statistics(time);

        Ok(ans)
    }
}

impl ImageToText<RgbImage> for YasOCRModel {
    fn image_to_text(&self, image: &RgbImage, is_preprocessed: bool) -> Result<String> {
        assert_eq!(is_preprocessed, false);

        let gray_image_float = preprocess::to_gray(image);
        let (result, non_mono) = preprocess::pre_process(gray_image_float);

        if !non_mono {
            return Ok(String::new());
        }

        let string_result = self.inference_string(&result)?;

        Ok(string_result)
    }
}

impl ImageToText<ImageBuffer<Luma<f32>, Vec<f32>>> for YasOCRModel {
    fn image_to_text(&self, image: &ImageBuffer<Luma<f32>, Vec<f32>>, is_preprocessed: bool) -> Result<String> {
        if is_preprocessed {
            let string_result = self.inference_string(image)?;
            Ok(string_result)
        } else {
            let im = image.clone();
            let (preprocess_result, non_mono) = preprocess::pre_process(im);

            if !non_mono {
                return Ok(String::new());
            }

            let string_result = self.inference_string(&preprocess_result)?;
            Ok(string_result)
        }
    }
}

impl ImageToText<GrayImage> for YasOCRModel {
    fn image_to_text(&self, im: &GrayImage, is_preprocessed: bool) -> Result<String> {
        let gray_f32_image: ImageBuffer<Luma<f32>, Vec<f32>> = im.to_f32_gray_image();
        self.image_to_text(&gray_f32_image, is_preprocessed)
    }
}

pub macro yas_ocr_model($model_name:literal, $index_to_word:literal) {
    {
        let model_bytes = include_bytes!($model_name);
        let index_to_word = include_str!($index_to_word);

        YasOCRModel::new(
            model_bytes, index_to_word,
        )
    }
}
